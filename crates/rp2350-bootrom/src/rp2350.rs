//! RP2350 synthetic boot ROM layout and trap semantics
//! RP2350 合成 Boot ROM 布局与陷阱语义
//!
//! Header fields follow the pico-sdk `bootrom_constants.h` offsets: the magic
//! word at `0x10` (whose third byte is the ROM version), the function table
//! offset at `0x14` and the `rom_table_lookup` pointer at `0x16`. Everything the
//! guest can call lives in the table at `0x7C00`; each pointer targets a `udf`
//! stub that the host dispatcher intercepts.
//! 头部字段遵循 pico-sdk `bootrom_constants.h` 的偏移：`0x10` 的 magic 字
//! （第三个字节是 ROM 版本）、`0x14` 的函数表偏移、`0x16` 的
//! `rom_table_lookup` 指针。客户机可调用的一切都位于 `0x7C00` 的表内，每个
//! 指针指向宿主分发器拦截的 `udf` 占位桩。

// 本模块用 16 位表指针、32 位地址与周期计数，类型窄化是布局定义的一部分。
// This module works with 16-bit table pointers, 32-bit addresses and cycle
// counts; the narrowing casts are part of the layout definition.
#![allow(clippy::cast_possible_truncation)]

use std::sync::Arc;

use rp2350_core::FlashHandle;
use rp2350_core::bus::{AddressBus, BusResult};
use rp2350_core::error::CoreError;
use rp2350_core::types::Address;

use crate::image::RomImageBuilder;
use crate::region::RomRegion;
use crate::table::{self, RomTableBuilder, TableCode, code};
use crate::trap::{HaltReason, TrapCall, TrapDispatcher, TrapHandler, TrapId, TrapOutcome};

/// Base address of the RP2350 boot ROM / RP2350 boot ROM 基地址
pub const ROM_BASE: Address = 0x0000_0000;
/// Size of the RP2350 boot ROM window (32 KB) / RP2350 boot ROM 窗口大小（32 KB）
pub const ROM_SIZE: usize = 0x8000;
/// Boot ROM magic word: `'M'`, `'u'`, magic version 2 and API version 2
/// Boot ROM magic 字：`'M'`、`'u'`、magic 版本 2 与 API 版本 2
///
/// The low three bytes `4D 75 02` match the vendor magic checked by debuggers;
/// byte `0x13` carries the boot ROM API version, which the flash-resident
/// helpers read directly and compare against 2.
/// 低三字节 `4D 75 02` 与调试器校验的厂商 magic 一致；字节 `0x13` 是 boot ROM
/// API 版本，Flash 内的辅助代码会直接读取并与 2 比较。
pub const ROM_MAGIC: u32 = 0x0202_754D;

/// Offset of the magic word / magic 字的偏移
const MAGIC_OFFSET: usize = 0x10;
/// Offset of the function table pointer / 函数表指针的偏移
const FUNC_TABLE_PTR_OFFSET: usize = 0x14;
/// Offset of the `rom_table_lookup` pointer / `rom_table_lookup` 指针的偏移
const LOOKUP_PTR_OFFSET: usize = 0x16;
/// Address of the `rom_table_lookup` stub / `rom_table_lookup` 占位桩地址
const LOOKUP_STUB: u32 = 0x74;
/// First function stub address / 首个函数占位桩地址
const STUB_BASE: u32 = 0x80;
/// Offset of the function/data table / 函数/数据表的偏移
const TABLE_OFFSET: usize = 0x7C00;
/// Offset of the flash device-info word / Flash 设备信息字的偏移
const DEVINFO_OFFSET: usize = 0x7D00;
/// Flash device info: CS0 = 16 MB, CS1 absent / Flash 设备信息：CS0 = 16 MB，无 CS1
const DEVINFO_VALUE: u32 = 0x0000_C000;

/// Trap identifier of the `rom_table_lookup` stub / `rom_table_lookup` 占位桩的陷阱标识
const TRAP_LOOKUP: u16 = 0;
/// `SYS_INFO_CHIP_INFO` / 芯片信息
const SYS_INFO_CHIP_INFO: u32 = 0x0001;
/// `SYS_INFO_CRITICAL` / 关键信息
const SYS_INFO_CRITICAL: u32 = 0x0002;
/// `SYS_INFO_CPU_INFO` / CPU 信息
const SYS_INFO_CPU_INFO: u32 = 0x0004;
/// `SYS_INFO_FLASH_DEV_INFO` / Flash 设备信息
const SYS_INFO_FLASH_DEV_INFO: u32 = 0x0008;
/// `SYS_INFO_BOOT_RANDOM` / 启动随机数
const SYS_INFO_BOOT_RANDOM: u32 = 0x0010;
/// `SYS_INFO_NONCE` / 随机数种子
const SYS_INFO_NONCE: u32 = 0x0020;
/// `SYS_INFO_BOOT_INFO` / 启动信息
const SYS_INFO_BOOT_INFO: u32 = 0x0040;
/// Union of the `SYS_INFO_*` queries this ROM answers / 本 ROM 支持的 `SYS_INFO_*` 查询集合
const SYS_INFO_SUPPORTED: u32 = SYS_INFO_CHIP_INFO
    | SYS_INFO_CRITICAL
    | SYS_INFO_CPU_INFO
    | SYS_INFO_FLASH_DEV_INFO
    | SYS_INFO_BOOT_RANDOM
    | SYS_INFO_NONCE
    | SYS_INFO_BOOT_INFO;

/// CPU info word: Arm (type 0) with the Arm bit set in the support bitfield
/// CPU 信息字：Arm（类型 0），支持位域中置 Arm 位
const CPU_INFO_WORD: u32 = 0x0000_0100;
/// Boot info word with `BOOT_TYPE_NORMAL` / `BOOT_TYPE_NORMAL` 的启动信息字
const BOOT_WORD: u32 = 0;

/// `BOOTROM_ERROR_BUFFER_TOO_SMALL` returned by `rom_get_sys_info`
/// `rom_get_sys_info` 返回的 `BOOTROM_ERROR_BUFFER_TOO_SMALL`
const ERROR_BUFFER_TOO_SMALL: u32 = 0xFFFF_FFF3;
/// `BOOTROM_ERROR_INVALID_ARG` returned for unknown traps
/// 未知陷阱返回的 `BOOTROM_ERROR_INVALID_ARG`
const ERROR_INVALID_ARG: u32 = 0xFFFF_FFEC;

/// Every ROM function this image exposes / 本镜像暴露的所有 ROM 函数
const ALL_TRAPS: [Trap; 8] = [
    Trap::ConnectFlash,
    Trap::ExitXip,
    Trap::RangeErase,
    Trap::RangeProgram,
    Trap::FlushCache,
    Trap::SysInfo,
    Trap::Reboot,
    Trap::StateReset,
];

/// A ROM function implemented by the host / 由宿主实现的 ROM 函数
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Trap {
    /// `rom_connect_internal_flash` / 连接内部 Flash
    ConnectFlash,
    /// `rom_flash_exit_xip` / 退出 XIP
    ExitXip,
    /// `rom_flash_range_erase` / 范围擦除
    RangeErase,
    /// `rom_flash_range_program` / 范围编程
    RangeProgram,
    /// `rom_flash_flush_cache` / 刷新缓存
    FlushCache,
    /// `rom_get_sys_info` / 获取系统信息
    SysInfo,
    /// `rom_reboot` / 重启
    Reboot,
    /// `rom_bootrom_state_reset` / 复位 boot ROM 状态
    StateReset,
}

impl Trap {
    /// Trap identifier carried by the stub / 占位桩携带的陷阱标识
    const fn id(self) -> u16 {
        match self {
            Self::ConnectFlash => 1,
            Self::ExitXip => 2,
            Self::RangeErase => 3,
            Self::RangeProgram => 4,
            Self::FlushCache => 5,
            Self::SysInfo => 6,
            Self::Reboot => 7,
            Self::StateReset => 8,
        }
    }

    /// Decode a trap identifier / 解码陷阱标识
    const fn from_id(id: u16) -> Option<Self> {
        match id {
            1 => Some(Self::ConnectFlash),
            2 => Some(Self::ExitXip),
            3 => Some(Self::RangeErase),
            4 => Some(Self::RangeProgram),
            5 => Some(Self::FlushCache),
            6 => Some(Self::SysInfo),
            7 => Some(Self::Reboot),
            8 => Some(Self::StateReset),
            _ => None,
        }
    }

    /// Table tag of the function / 函数的表标签
    const fn tag(self) -> TableCode {
        match self {
            Self::ConnectFlash => code(b'I', b'F'),
            Self::ExitXip => code(b'E', b'X'),
            Self::RangeErase => code(b'R', b'E'),
            Self::RangeProgram => code(b'R', b'P'),
            Self::FlushCache => code(b'F', b'C'),
            Self::SysInfo => code(b'G', b'S'),
            Self::Reboot => code(b'R', b'B'),
            Self::StateReset => code(b'S', b'R'),
        }
    }

    /// Thumb stub address in the image / 镜像中的 Thumb 占位桩地址
    const fn stub(self) -> u32 {
        STUB_BASE + 2 * (self.id() - 1) as u32
    }
}

/// A mounted RP2350 synthetic boot ROM / 已挂载的 RP2350 合成 boot ROM
pub struct Rp2350Bootrom {
    image: Arc<Vec<u8>>,
    dispatcher: TrapDispatcher,
}

impl Rp2350Bootrom {
    /// Build the synthetic image and its trap dispatcher
    /// 构建合成镜像及其陷阱分发器
    ///
    /// `flash` is the host handle used by the flash erase/program functions. It
    /// writes the same backing store the guest reads through the XIP windows.
    /// `flash` 是 Flash 擦除/编程函数使用的宿主句柄，写入客户机经 XIP 窗口读取
    /// 的同一份底层存储。
    #[must_use]
    pub fn new(flash: FlashHandle) -> Self {
        let image = Arc::new(build_image());
        let handler = Arc::new(Rp2350TrapHandler {
            image: Arc::clone(&image),
            flash,
            boot_random: [0x1234_5678, 0x9ABC_DEF0, 0x0F1E_2D3C, 0x4B5A_6978],
        });
        let mut dispatcher = TrapDispatcher::new(handler);
        dispatcher.register(LOOKUP_STUB, TrapId(TRAP_LOOKUP));
        for trap in ALL_TRAPS {
            dispatcher.register(trap.stub(), TrapId(trap.id()));
        }
        Self { image, dispatcher }
    }

    /// Borrow the built image / 借用已构建的镜像
    #[must_use]
    pub fn image(&self) -> &[u8] {
        &self.image
    }

    /// Register the read-only ROM window on `bus` / 在 `bus` 上注册只读 ROM 窗口
    ///
    /// # Errors
    /// Returns a bus error if another region overlaps `0x0000_0000..0x0000_8000`.
    /// 若其他区域与 `0x0000_0000..0x0000_8000` 重叠，返回总线错误。
    pub fn mount(&self, bus: &mut dyn AddressBus) -> BusResult<()> {
        bus.register_region(Box::new(RomRegion::new(
            "bootrom",
            ROM_BASE,
            Arc::clone(&self.image),
        )))
    }

    /// Report whether `pc` is a trap stub / 报告 `pc` 是否为陷阱占位桩
    #[must_use]
    pub fn handles(&self, pc: u32) -> bool {
        self.dispatcher.handles(pc)
    }

    /// Dispatch the trap at the core's current PC / 分发核心当前 PC 处的陷阱
    ///
    /// # Errors
    /// Propagates bus errors from the chip handler.
    /// 传播芯片处理器产生的总线错误。
    pub fn dispatch(
        &self,
        core: &mut dyn rp2350_cpu::CpuCore,
        bus: &mut dyn AddressBus,
    ) -> Result<Option<TrapOutcome>, CoreError> {
        self.dispatcher.dispatch(core, bus)
    }
}

/// Build the 32 KB synthetic image / 构建 32 KB 合成镜像
fn build_image() -> Vec<u8> {
    let mut image = RomImageBuilder::new(ROM_SIZE);
    image.write_u32(MAGIC_OFFSET, ROM_MAGIC);
    image.write_u16(FUNC_TABLE_PTR_OFFSET, TABLE_OFFSET as u16);
    image.write_u16(LOOKUP_PTR_OFFSET, (LOOKUP_STUB as u16) | 1);
    image.trap_stub(LOOKUP_STUB as usize, TRAP_LOOKUP);
    for trap in ALL_TRAPS {
        image.trap_stub(trap.stub() as usize, trap.id());
    }

    let mut table = RomTableBuilder::new();
    for trap in ALL_TRAPS {
        let pointer = (trap.stub() as u16) | 1;
        table.push_func(trap.tag(), pointer, pointer);
    }
    table.push_data(code(b'F', b'D'), DEVINFO_OFFSET as u16);
    image.write_slice(TABLE_OFFSET, &table.finish());
    image.write_u32(DEVINFO_OFFSET, DEVINFO_VALUE);
    image.finish()
}

/// Chip-specific trap implementation / 芯片相关的陷阱实现
struct Rp2350TrapHandler {
    image: Arc<Vec<u8>>,
    flash: FlashHandle,
    boot_random: [u32; 4],
}

impl Rp2350TrapHandler {
    /// Answer `rom_table_lookup(code, mask)` / 应答 `rom_table_lookup(code, mask)`
    fn table_lookup(&self, args: [u32; 4]) -> u32 {
        let code = args[0] as u16;
        let mask = args[1] as u16;
        table::lookup(&self.image[TABLE_OFFSET..], code, mask).map_or(0, u32::from)
    }

    /// Erase `count` bytes rounded up to `block_size` / 按 `block_size` 上取整擦除 `count` 字节
    fn range_erase(&self, addr: u32, count: u32, block_size: u32) -> Result<(), CoreError> {
        let block = block_size.max(1);
        let span = count.div_ceil(block).saturating_mul(block);
        self.flash.erase(addr, span as usize)
    }

    /// Program `count` bytes copied from guest memory / 编程从客户机内存复制的 `count` 字节
    fn range_program(
        &self,
        addr: u32,
        source: u32,
        count: u32,
        bus: &mut dyn AddressBus,
        cycles: &mut u64,
    ) -> Result<(), CoreError> {
        let mut bytes = Vec::with_capacity(count as usize);
        for offset in 0..count {
            bytes.push(bus.read_u8(source.wrapping_add(offset), cycles)?);
        }
        self.flash.write(addr, &bytes)
    }

    /// Answer `rom_get_sys_info(buffer, capacity, flags)`
    /// 应答 `rom_get_sys_info(buffer, capacity, flags)`
    fn sys_info(
        &self,
        buffer: u32,
        capacity: u32,
        flags: u32,
        bus: &mut dyn AddressBus,
        cycles: &mut u64,
    ) -> Result<u32, CoreError> {
        let supported = flags & SYS_INFO_SUPPORTED;
        let mut words = vec![supported];
        for bit in 0..7 {
            let flag = 1u32 << bit;
            if supported & flag == 0 {
                continue;
            }
            match flag {
                SYS_INFO_CHIP_INFO => words.extend_from_slice(&[0, 0, 0]),
                SYS_INFO_CRITICAL => words.push(0),
                SYS_INFO_CPU_INFO => words.push(CPU_INFO_WORD),
                SYS_INFO_FLASH_DEV_INFO => words.push(DEVINFO_VALUE),
                SYS_INFO_BOOT_RANDOM => words.extend_from_slice(&self.boot_random),
                SYS_INFO_NONCE => words.extend_from_slice(&[0, 0]),
                SYS_INFO_BOOT_INFO => words.extend_from_slice(&[BOOT_WORD, 0, 0, 0]),
                _ => {}
            }
        }

        let needed = words.len() as u32;
        if capacity < needed {
            return Ok(ERROR_BUFFER_TOO_SMALL);
        }
        for (index, word) in words.iter().enumerate() {
            bus.write_u32(buffer.wrapping_add((index as u32) * 4), *word, cycles)?;
        }
        Ok(needed)
    }
}

impl TrapHandler for Rp2350TrapHandler {
    fn invoke(
        &self,
        call: TrapCall,
        bus: &mut dyn AddressBus,
        cycles: &mut u64,
    ) -> Result<TrapOutcome, CoreError> {
        if call.id.0 == TRAP_LOOKUP {
            return Ok(TrapOutcome::value(self.table_lookup(call.args), 8));
        }

        let [addr, arg1, arg2, arg3] = call.args;
        let outcome = match Trap::from_id(call.id.0) {
            Some(Trap::ConnectFlash | Trap::ExitXip) => TrapOutcome::void(32),
            Some(Trap::FlushCache) => TrapOutcome::void(16),
            Some(Trap::RangeErase) => {
                self.range_erase(addr, arg1, arg2)?;
                TrapOutcome::void(arg1.div_ceil(256).max(64))
            }
            Some(Trap::RangeProgram) => {
                self.range_program(addr, arg1, arg2, bus, cycles)?;
                TrapOutcome::void(arg2.div_ceil(16).max(16))
            }
            Some(Trap::SysInfo) => {
                TrapOutcome::value(self.sys_info(addr, arg1, arg2, bus, cycles)?, 64)
            }
            Some(Trap::Reboot) => TrapOutcome {
                result: 0,
                cycles: 8,
                halt: Some(HaltReason::Reboot {
                    flags: addr,
                    delay_ms: arg1,
                    p0: arg2,
                    p1: arg3,
                }),
            },
            Some(Trap::StateReset) => TrapOutcome::void(8),
            None => TrapOutcome::value(ERROR_INVALID_ARG, 4),
        };
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::table::flags;
    use rp2350_core::bus::AddressBus;
    use rp2350_core::{FLASH_XIP_BASE, FlashRegion, RamRegion, SimpleBus};

    /// Test SRAM base address / 测试用 SRAM 基地址
    const SRAM_BASE: u32 = 0x2000_0000;

    fn bootrom() -> Rp2350Bootrom {
        let flash = FlashRegion::rp2350_xip();
        Rp2350Bootrom::new(flash.handle())
    }

    #[test]
    fn header_matches_the_documented_layout() {
        let rom = bootrom();
        let image = rom.image();
        assert_eq!(&image[0x10..0x14], &ROM_MAGIC.to_le_bytes());
        assert_eq!(image[0x12], 2);
        assert_eq!(image[0x13], 2);
        assert_eq!(u16::from_le_bytes([image[0x14], image[0x15]]), 0x7C00);
        let lookup = u16::from_le_bytes([image[0x16], image[0x17]]);
        assert_eq!(lookup & 1, 1);
        assert_eq!(u32::from(lookup & !1), LOOKUP_STUB);
    }

    #[test]
    fn table_finds_every_function_and_the_devinfo() {
        let rom = bootrom();
        let image = rom.image();
        let table = &image[TABLE_OFFSET..];
        for trap in ALL_TRAPS {
            assert_eq!(
                table::lookup(table, trap.tag(), flags::FUNC_ARM_SEC),
                Some((trap.stub() as u16) | 1)
            );
        }
        assert_eq!(
            table::lookup(table, code(b'F', b'D'), flags::DATA),
            Some(DEVINFO_OFFSET as u16)
        );
        assert_eq!(
            u32::from_le_bytes(
                image[DEVINFO_OFFSET..DEVINFO_OFFSET + 4]
                    .try_into()
                    .unwrap()
            ),
            DEVINFO_VALUE
        );
    }

    #[test]
    fn every_stub_is_registered() {
        let rom = bootrom();
        assert!(rom.handles(LOOKUP_STUB));
        for trap in ALL_TRAPS {
            assert!(rom.handles(trap.stub()));
        }
        assert!(!rom.handles(STUB_BASE + 0x40));
    }

    #[test]
    fn sys_info_reports_boot_info() {
        let flash = FlashRegion::rp2350_xip();
        let handler = Rp2350TrapHandler {
            image: Arc::new(build_image()),
            flash: flash.handle(),
            boot_random: [1, 2, 3, 4],
        };
        let mut bus = SimpleBus::new();
        bus.register_region(Box::new(RamRegion::new("sram", SRAM_BASE, 0x1000)))
            .unwrap();
        let mut cycles = 0u64;

        let outcome = handler
            .invoke(
                TrapCall {
                    id: TrapId(Trap::SysInfo.id()),
                    args: [SRAM_BASE, 5, SYS_INFO_BOOT_INFO, 0],
                },
                &mut bus,
                &mut cycles,
            )
            .unwrap();
        assert_eq!(outcome.result, 5);
        assert_eq!(
            bus.read_u32(SRAM_BASE, &mut cycles).unwrap(),
            SYS_INFO_BOOT_INFO
        );
        assert_eq!(bus.read_u32(SRAM_BASE + 4, &mut cycles).unwrap(), BOOT_WORD);
    }

    #[test]
    fn flash_program_and_erase_share_the_guest_store() {
        let flash = FlashRegion::rp2350_xip();
        let handle = flash.handle();
        let handler = Rp2350TrapHandler {
            image: Arc::new(build_image()),
            flash: handle.clone(),
            boot_random: [0; 4],
        };
        let mut bus = SimpleBus::new();
        bus.register_region(Box::new(RamRegion::new("sram", SRAM_BASE, 0x100)))
            .unwrap();
        let mut cycles = 0u64;
        for (index, byte) in [0xDE_u8, 0xAD, 0xBE, 0xEF].iter().enumerate() {
            bus.write_u8(SRAM_BASE + index as u32, *byte, &mut cycles)
                .unwrap();
        }

        handler
            .invoke(
                TrapCall {
                    id: TrapId(Trap::RangeProgram.id()),
                    args: [FLASH_XIP_BASE, SRAM_BASE, 4, 0],
                },
                &mut bus,
                &mut cycles,
            )
            .unwrap();
        assert_eq!(
            handle.read(FLASH_XIP_BASE, 4).unwrap(),
            vec![0xDE, 0xAD, 0xBE, 0xEF]
        );

        handler
            .invoke(
                TrapCall {
                    id: TrapId(Trap::RangeErase.id()),
                    args: [FLASH_XIP_BASE, 4, 4, 0xD8],
                },
                &mut bus,
                &mut cycles,
            )
            .unwrap();
        assert_eq!(handle.read(FLASH_XIP_BASE, 4).unwrap(), vec![0xFF; 4]);
    }
}
