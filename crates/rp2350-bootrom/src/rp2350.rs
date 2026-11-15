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
/// Offset of the 32-bit pointer literal that locates the device-info word
/// 指向设备信息字的 32 位指针字面量的偏移
///
/// `rom_data_lookup` returns a pointer to this literal, not the word itself:
/// `flash_devinfo_ptr()` dereferences it once to reach `DEVINFO_OFFSET`.
/// `rom_data_lookup` 返回的是指向该字面量的指针，而非字本身：
/// `flash_devinfo_ptr()` 对它解引用一次才得到 `DEVINFO_OFFSET` 处的 16 位字。
const DEVINFO_PTR_OFFSET: usize = 0x7D04;
/// RISC-V slot holding the `rom_func_lookup` entry point / 存放 RISC-V `rom_func_lookup` 入口的槽位
///
/// The RP2350 RISC-V boot2 loads a 16-bit pointer from `0x7DFA` and calls it
/// with `(code, mask)` in `a0`/`a1`. The real ROM stores `0x75C2` there; the
/// synthetic image mirrors that value so the boot2 finds a host trap stub.
/// RP2350 的 RISC-V boot2 从 `0x7DFA` 加载 16 位指针，并以 `a0`/`a1` 传入
/// `(code, mask)` 调用它。真实 ROM 在此存放 `0x75C2`；合成镜像沿用该值，使
/// boot2 命中宿主陷阱桩。
const RV_LOOKUP_PTR_OFFSET: usize = 0x7DFA;
/// Address of the RISC-V `rom_func_lookup` trap stub / RISC-V `rom_func_lookup` 陷阱桩地址
const RV_LOOKUP_STUB: u32 = 0x75C2;
/// RISC-V slot holding the `rom_data_lookup` value wrapper
/// 存放 RISC-V `rom_data_lookup` 取值包装的槽位
///
/// The real ROM exposes two adjacent halfword entry points: `0x7DFA` holds
/// `rom_func_lookup` (`0x75C2`) and `0x7DF8` holds `rom_data_lookup`
/// (`0x75FE`), which performs the table lookup and then loads the 16-bit
/// value. `MicroPython`'s SRAM flash helper reads the latter slot.
/// 真实 ROM 暴露两个相邻半字入口：`0x7DFA` 存放 `rom_func_lookup`
/// （`0x75C2`），`0x7DF8` 存放 `rom_data_lookup`（`0x75FE`），后者先查表再
/// 读取 16 位值。MicroPython 的 SRAM flash 辅助代码读取的是后者。
const RV_DATA_LOOKUP_PTR_OFFSET: usize = 0x7DF8;
/// Address of the RISC-V `rom_data_lookup` value wrapper
/// RISC-V `rom_data_lookup` 取值包装的地址
const RV_DATA_LOOKUP_STUB: u32 = 0x75FE;
/// First RISC-V function trap stub address / 首个 RISC-V 函数陷阱桩地址
const RV_STUB_BASE: u32 = 0x7600;
/// Distance between RISC-V function trap stubs / RISC-V 函数陷阱桩间距
const RV_STUB_STRIDE: u32 = 0x10;
/// First address of the zero-returning RISC-V stubs
/// 返回 0 的 RISC-V 占位桩起始地址
///
/// These back the RP2350 ROM functions this image does not implement. The
/// pointer must be valid (so `rom_func_lookup` succeeds and a caller's NULL
/// check does not misfire), but the call itself returns 0 and has no effect.
/// 这些桩服务于本镜像未实现的 RP2350 ROM 函数。指针必须有效（使
/// `rom_func_lookup` 成功、调用者的 NULL 检查不会误判），但调用本身返回 0
/// 且没有副作用。
const RV_ZERO_STUB_BASE: u32 = 0x7700;
/// Distance between zero-returning RISC-V stubs / 返回 0 的 RISC-V 占位桩间距
const RV_ZERO_STUB_STRIDE: u32 = 0x10;
/// Offset of the 32-bit zero literal named by the RISC-V data tags
/// RISC-V 数据标签所指向的 32 位零字面量的偏移
const RV_ZERO_DATA_PTR_OFFSET: usize = 0x7D08;

/// Base of the synthetic RISC-V fatal-exception vector table
/// 合成 RISC-V 致命异常向量表基地址
///
/// The table lives in the unused tail of the 32 KB ROM window. Each slot is a
/// `jal x0` to [`RISCV_EXCEPTION_HANDLER`], so it is executable both by the
/// host dispatcher and by a real core when no host intercept is installed.
/// 该表位于 32 KB ROM 窗口末尾的未使用区域。每个槽位都是跳向
/// [`RISCV_EXCEPTION_HANDLER`] 的 `jal x0`，既可被宿主分发器拦截，也可在未安装
/// 宿主拦截时由真实核心直接执行。
pub const RISCV_EXCEPTION_VECTOR_BASE: Address = 0x0000_7E00;
/// Common diagnostic handler for the synthetic RISC-V exception vectors
/// 合成 RISC-V 异常向量的公共诊断处理程序
pub const RISCV_EXCEPTION_HANDLER: Address = 0x0000_7F00;
/// Exception causes routed through the synthetic fatal vector
/// 经由合成致命向量路由的异常原因
///
/// At least illegal instruction (2) and load access fault (5); both are fatal
/// during bring-up and would otherwise loop through an unfilled guest vector.
/// 至少包含非法指令（2）与加载访问错误（5）；两者在启动阶段都是致命的，否则
/// 会在固件未填充的向量槽内循环。
pub const RISCV_FATAL_CAUSES: [u32; 2] = [2, 5];
/// Number of RISC-V exception vector slots written to the image
/// 镜像中写入的 RISC-V 异常向量槽数量
const RV_EXCEPTION_VECTOR_SLOTS: u32 = 16;
/// Trap identifier of the RISC-V `rom_func_lookup` stub / RISC-V `rom_func_lookup` 占位桩的陷阱标识
const TRAP_RV_LOOKUP: u16 = 0x0100;
/// Trap identifier of the RISC-V `rom_data_lookup` value wrapper
/// RISC-V `rom_data_lookup` 取值包装的陷阱标识
const TRAP_RV_DATA_LOOKUP: u16 = 0x0101;
/// Trap identifier of the zero-returning RISC-V stubs
/// 返回 0 的 RISC-V 占位桩的陷阱标识
const TRAP_RV_ZERO_STUB: u16 = 0x0102;

/// RP2350 RISC-V function tags that resolve to a zero-returning stub
/// 解析为返回 0 占位桩的 RP2350 RISC-V 函数标签
///
/// Every tag below exists in the real RP2350 `bootrom_constants.h` table; this
/// image only implements the flash helpers and `rom_get_sys_info` for real, so
/// the rest map to stubs. Tags already implemented by [`Trap`] are listed there,
/// not here. / 以下标签都存在于真实 RP2350 的 `bootrom_constants.h` 表中；本镜像
/// 只真实实现 flash 辅助函数与 `rom_get_sys_info`，其余映射为桩。已由 [`Trap`]
/// 实现的标签列在那里，不在此处。
const RV_STUB_FUNC_TAGS: [TableCode; 18] = [
    code(b'A', b'B'), // _pick_ab_partition
    code(b'C', b'I'), // _chain_image
    code(b'E', b'B'), // _explicit_buy
    code(b'F', b'A'), // _flash_runtime_to_storage_addr
    code(b'F', b'O'), // _flash_op
    code(b'G', b'B'), // _get_b_partition
    code(b'G', b'P'), // _get_partition_table_info
    code(b'G', b'U'), // _get_uf2_target_partition
    code(b'L', b'P'), // _load_partition_table
    code(b'O', b'A'), // _otp_access
    code(b'R', b'A'), // _flash_reset_address_trans
    code(b'R', b'C'), // _set_rom_callback
    code(b'S', b'C'), // _secure_call
    code(b'S', b'P'), // _set_ns_api_permission
    code(b'S', b'S'), // _set_bootrom_stack
    code(b'U', b'B'), // _reset_to_usb_boot (RP2040 name, still in the ROM)
    code(b'X', b'M'), // _flash_select_xip_read_mode
    code(b'V', b'B'), // _validate_ns_buffer
];

/// RP2350 RISC-V data tags that resolve to a zero literal
/// 解析为零字面量的 RP2350 RISC-V 数据标签
const RV_STUB_DATA_TAGS: [TableCode; 3] = [
    code(b'G', b'R'), // _software_git_revision
    code(b'P', b'T'), // _partition_table
    code(b'X', b'F'), // _saved_xip_setup_func_ptr
];
/// Flash device info: CS0 = 16 MB, CS1 absent / Flash 设备信息：CS0 = 16 MB，无 CS1
///
/// pico-sdk `hardware/flash.h` encodes sizes as `4096 << code` with
/// `FLASH_DEVINFO_SIZE_MAX == 0xC` (16 MB); `OTP_DATA_FLASH_DEVINFO_CS0_SIZE_BITS`
/// occupies bits 11:8, so the word is `0x0C00`.
/// pico-sdk `hardware/flash.h` 以 `4096 << code` 编码容量，`FLASH_DEVINFO_SIZE_MAX`
/// 即 `0xC`（16 MB）；`OTP_DATA_FLASH_DEVINFO_CS0_SIZE_BITS` 位于 bits 11:8，
/// 因此该字应为 `0x0C00`。
const DEVINFO_VALUE: u16 = 0x0C00;

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
const ALL_TRAPS: [Trap; 9] = [
    Trap::ConnectFlash,
    Trap::ExitXip,
    Trap::RangeErase,
    Trap::RangeProgram,
    Trap::FlushCache,
    Trap::EnterCmdXip,
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
    /// `rom_flash_enter_cmd_xip` / 进入命令模式 XIP
    EnterCmdXip,
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
            Self::EnterCmdXip => 6,
            Self::SysInfo => 7,
            Self::Reboot => 8,
            Self::StateReset => 9,
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
            6 => Some(Self::EnterCmdXip),
            7 => Some(Self::SysInfo),
            8 => Some(Self::Reboot),
            9 => Some(Self::StateReset),
            _ => None,
        }
    }

    /// Table tag of the function / 函数的表标签
    ///
    /// RISC-V boot2 looks up `IX` for `rom_flash_exit_xip` while the Arm SDK
    /// uses `EX`; [`Self::from_tag`] accepts both.
    /// RISC-V boot2 用 `IX` 查询 `rom_flash_exit_xip`，而 Arm SDK 使用 `EX`；
    /// [`Self::from_tag`] 两者都接受。
    const fn tag(self) -> TableCode {
        match self {
            Self::ConnectFlash => code(b'I', b'F'),
            Self::ExitXip => code(b'E', b'X'),
            Self::RangeErase => code(b'R', b'E'),
            Self::RangeProgram => code(b'R', b'P'),
            Self::FlushCache => code(b'F', b'C'),
            Self::EnterCmdXip => code(b'C', b'X'),
            Self::SysInfo => code(b'G', b'S'),
            Self::Reboot => code(b'R', b'B'),
            Self::StateReset => code(b'S', b'R'),
        }
    }

    /// Decode a boot ROM function tag / 解码 boot ROM 函数标签
    const fn from_tag(tag: TableCode) -> Option<Self> {
        match tag {
            0x4649 => Some(Self::ConnectFlash),     // IF
            0x5845 | 0x5849 => Some(Self::ExitXip), // EX / IX
            0x4552 => Some(Self::RangeErase),       // RE
            0x5052 => Some(Self::RangeProgram),     // RP
            0x4346 => Some(Self::FlushCache),       // FC
            0x5843 => Some(Self::EnterCmdXip),      // CX
            0x5347 => Some(Self::SysInfo),          // GS
            0x4252 => Some(Self::Reboot),           // RB
            0x5253 => Some(Self::StateReset),       // SR
            _ => None,
        }
    }

    /// Thumb stub address in the image / 镜像中的 Thumb 占位桩地址
    const fn stub(self) -> u32 {
        STUB_BASE + 2 * (self.id() - 1) as u32
    }

    /// RISC-V stub address in the image / 镜像中的 RISC-V 占位桩地址
    const fn rv_stub(self) -> u32 {
        RV_STUB_BASE + RV_STUB_STRIDE * (self.id() - 1) as u32
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
        dispatcher.register(RV_LOOKUP_STUB, TrapId(TRAP_RV_LOOKUP));
        dispatcher.register(RV_DATA_LOOKUP_STUB, TrapId(TRAP_RV_DATA_LOOKUP));
        for trap in ALL_TRAPS {
            dispatcher.register(trap.stub(), TrapId(trap.id()));
            dispatcher.register(trap.rv_stub(), TrapId(trap.id()));
        }
        for index in 0..RV_STUB_FUNC_TAGS.len() {
            dispatcher.register(rv_zero_stub(index), TrapId(TRAP_RV_ZERO_STUB));
        }
        Self { image, dispatcher }
    }

    /// Borrow the built image / 借用已构建的镜像
    #[must_use]
    pub fn image(&self) -> &[u8] {
        &self.image
    }

    /// Return the synthetic RISC-V fatal-exception vector base
    /// 返回合成 RISC-V 致命异常向量基地址
    ///
    /// Hosts install this with `Hazard3Core::set_fatal_trap_vector` so causes
    /// 2 and 5 reach the diagnostic handler instead of a guest vector slot.
    /// 宿主应通过 `Hazard3Core::set_fatal_trap_vector` 安装该地址，使 cause 2 与
    /// 5 进入诊断处理程序而非客户机向量槽。
    #[must_use]
    pub const fn riscv_exception_vector(&self) -> Address {
        RISCV_EXCEPTION_VECTOR_BASE
    }

    /// Report whether `pc` is a synthetic RISC-V fatal-exception vector slot
    /// 报告 `pc` 是否为合成 RISC-V 致命异常向量槽
    #[must_use]
    fn is_riscv_exception_vector(pc: u32) -> bool {
        RISCV_FATAL_CAUSES
            .iter()
            .any(|cause| pc == RISCV_EXCEPTION_VECTOR_BASE.wrapping_add(4 * cause))
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

    /// Report whether `pc` is a trap stub or a fatal-exception vector slot
    /// 报告 `pc` 是否为陷阱占位桩或致命异常向量槽
    #[must_use]
    pub fn handles(&self, pc: u32) -> bool {
        Self::is_riscv_exception_vector(pc) || self.dispatcher.handles(pc)
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
        let pc = core.pc();
        if Self::is_riscv_exception_vector(pc)
            && matches!(core.arch(), rp2350_cpu::CpuArch::RiscVHazard3)
        {
            let mcause = core.csr_read(0x342);
            let mepc = core.csr_read(0x341);
            let mtval = core.csr_read(0x343);
            // Keep the post-mortem PC on the faulting instruction so the
            // final report points into Flash/SRAM rather than the ROM vector.
            // 把停机 PC 留在出错指令上，使最终报告指向 Flash/SRAM 而不是 ROM 向量。
            core.set_pc(mepc);
            return Ok(Some(TrapOutcome {
                result: 0,
                cycles: 4,
                halt: Some(HaltReason::RiscvException {
                    mcause,
                    mepc,
                    mtval,
                }),
            }));
        }
        self.dispatcher.dispatch(core, bus)
    }
}

/// Encode `jal x0, target` for a RISC-V exception vector slot
/// 为 RISC-V 异常向量槽编码 `jal x0, target`
fn riscv_jal_x0(from: u32, to: u32) -> u32 {
    let value = to.wrapping_sub(from);
    0x6F | (((value >> 20) & 1) << 31)
        | (((value >> 1) & 0x3FF) << 21)
        | (((value >> 11) & 1) << 20)
        | (((value >> 12) & 0xFF) << 12)
}

/// Address of the zero-returning stub for [`RV_STUB_FUNC_TAGS`] entry `index`
/// 返回 0 的占位桩地址（对应 [`RV_STUB_FUNC_TAGS`] 的第 `index` 项）
fn rv_zero_stub(index: usize) -> u32 {
    RV_ZERO_STUB_BASE + RV_ZERO_STUB_STRIDE * index as u32
}

/// Build the 32 KB synthetic image / 构建 32 KB 合成镜像
fn build_image() -> Vec<u8> {
    let mut image = RomImageBuilder::new(ROM_SIZE);
    image.write_u32(MAGIC_OFFSET, ROM_MAGIC);
    image.write_u16(FUNC_TABLE_PTR_OFFSET, TABLE_OFFSET as u16);
    image.write_u16(LOOKUP_PTR_OFFSET, (LOOKUP_STUB as u16) | 1);
    image.trap_stub(LOOKUP_STUB as usize, TRAP_LOOKUP);
    // RISC-V boot2 reads the lookup entry point from a fixed halfword slot and
    // calls it directly; expose the same trap through that slot.
    // RISC-V boot2 从固定半字槽读取查询入口并直接调用；通过该槽暴露同一陷阱。
    image.write_u16(RV_LOOKUP_PTR_OFFSET, RV_LOOKUP_STUB as u16);
    image.riscv_trap_stub(RV_LOOKUP_STUB as usize);
    image.write_u16(RV_DATA_LOOKUP_PTR_OFFSET, RV_DATA_LOOKUP_STUB as u16);
    image.riscv_trap_stub(RV_DATA_LOOKUP_STUB as usize);
    for trap in ALL_TRAPS {
        image.trap_stub(trap.stub() as usize, trap.id());
        image.riscv_trap_stub(trap.rv_stub() as usize);
    }
    // 未实现的 RP2350 ROM 函数共用同一组「返回 0」桩；地址有效，调用无副作用。
    // Unimplemented RP2350 ROM functions share a set of zero-returning stubs:
    // the addresses are valid and the calls have no side effects.
    for index in 0..RV_STUB_FUNC_TAGS.len() {
        image.riscv_trap_stub(rv_zero_stub(index) as usize);
    }
    // RISC-V fatal-exception vectors: every slot jumps to the common handler,
    // which is a `ebreak` stub the host dispatcher intercepts.
    // RISC-V 致命异常向量：每个槽位跳到公共处理程序，后者是宿主分发器拦截的
    // `ebreak` 占位桩。
    for cause in 0..RV_EXCEPTION_VECTOR_SLOTS {
        let slot = RISCV_EXCEPTION_VECTOR_BASE + 4 * cause;
        image.write_u32(slot as usize, riscv_jal_x0(slot, RISCV_EXCEPTION_HANDLER));
    }
    image.riscv_trap_stub(RISCV_EXCEPTION_HANDLER as usize);

    let mut table = RomTableBuilder::new();
    for trap in ALL_TRAPS {
        let pointer = (trap.stub() as u16) | 1;
        // 同一表项同时给出 ARM 安全/非安全与 RISC-V 指针，与真实 ROM 布局一致。
        // One entry carries the ARM secure/non-secure and RISC-V pointers, as the
        // real ROM does.
        table.push_func_rv(trap.tag(), pointer, pointer, trap.rv_stub() as u16);
    }
    for (index, tag) in RV_STUB_FUNC_TAGS.iter().enumerate() {
        table.push_riscv_func(*tag, rv_zero_stub(index) as u16);
    }
    table.push_data(code(b'F', b'D'), DEVINFO_PTR_OFFSET as u16);
    for tag in RV_STUB_DATA_TAGS {
        table.push_data(tag, RV_ZERO_DATA_PTR_OFFSET as u16);
    }
    image.write_slice(TABLE_OFFSET, &table.finish());
    // 与真实 ROM 相同的两级间接：表项指向字面量，字面量再指向 16 位信息字。
    // Same two-level indirection as the real ROM: the entry points at the
    // literal, the literal points at the 16-bit info word.
    image.write_u32(DEVINFO_PTR_OFFSET, DEVINFO_OFFSET as u32);
    image.write_u16(DEVINFO_OFFSET, DEVINFO_VALUE);
    // 未实现的数据标签指向一个零字面量，使调用者拿到有效指针而非 NULL。
    // Unimplemented data tags point at a zero literal so callers get a valid
    // pointer instead of NULL.
    image.write_u32(RV_ZERO_DATA_PTR_OFFSET, 0);
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

    /// Answer the RISC-V `rom_func_lookup(code, mask)` call
    /// 应答 RISC-V `rom_func_lookup(code, mask)` 调用
    ///
    /// The real RISC-V ROM searches a table of 16-bit tags. Functions used by
    /// boot2 and the `MicroPython` runtime are host traps with real behaviour;
    /// the remaining RP2350 tags resolve to zero-returning stubs so callers see
    /// a valid pointer. Unknown tags return 0, which is the ROM's own "not
    /// found" result.
    /// 真实 RISC-V ROM 会搜索 16 位标签表。boot2 与 `MicroPython` 运行时使用的
    /// 函数是有真实行为的宿主陷阱；其余 RP2350 标签解析为返回 0 的占位桩，使
    /// 调用者拿到有效指针。未知标签返回 0，与 ROM 自身的「未找到」结果一致。
    fn rv_table_lookup(args: [u32; 4]) -> u32 {
        let tag = args[0] as u16;
        if let Some(trap) = Trap::from_tag(tag) {
            return trap.rv_stub();
        }
        RV_STUB_FUNC_TAGS
            .iter()
            .position(|candidate| *candidate == tag)
            .map_or(0, rv_zero_stub)
    }

    /// Answer the RISC-V `rom_data_lookup` wrapper / 应答 RISC-V `rom_data_lookup` 包装
    ///
    /// The real wrapper calls the table lookup and then loads the 16-bit value.
    /// Function tags return their trap stub directly; `FD` points at the 32-bit
    /// literal that in turn points at the 16-bit `FLASH_DEVINFO` word, matching
    /// the two-level indirection used by the Arm API. The unmodelled data tags
    /// point at a zero literal.
    /// 真实包装先查表再读取 16 位值。函数标签直接返回陷阱桩；`FD` 指向那个再
    /// 指向 16 位 `FLASH_DEVINFO` 字的 32 位字面量，与 Arm API 的两级间接一致。
    /// 未建模的数据标签指向零字面量。
    fn rv_data_lookup(args: [u32; 4]) -> u32 {
        let tag = args[0] as u16;
        if tag == code(b'F', b'D') {
            return DEVINFO_PTR_OFFSET as u32;
        }
        if RV_STUB_DATA_TAGS.contains(&tag) {
            return RV_ZERO_DATA_PTR_OFFSET as u32;
        }
        Self::rv_table_lookup(args)
    }

    /// Erase `count` bytes rounded up to `block_size` / 按 `block_size` 上取整擦除 `count` 字节
    ///
    /// The ROM API takes a flash offset, not an XIP address: `flash_range_erase`
    /// callers pass e.g. `0x0010_0000` for the second megabyte. Translate it to
    /// the backing store by adding the XIP base before erasing.
    /// ROM API 接受的是 Flash 偏移而非 XIP 地址：`flash_range_erase` 调用者会为
    /// 第二个 MB 传入例如 `0x0010_0000`。擦除前要加上 XIP 基地址，换算到共享
    /// 存储地址。
    fn range_erase(&self, offset: u32, count: u32, block_size: u32) -> Result<(), CoreError> {
        let block = block_size.max(1);
        let span = count.div_ceil(block).saturating_mul(block);
        self.flash
            .erase(self.flash.base().wrapping_add(offset), span as usize)
    }

    /// Program `count` bytes copied from guest memory / 编程从客户机内存复制的 `count` 字节
    ///
    /// Like [`Self::range_erase`], `offset` is relative to the start of flash.
    /// 与 [`Self::range_erase`] 相同，`offset` 相对于 Flash 起始地址。
    fn range_program(
        &self,
        offset: u32,
        source: u32,
        count: u32,
        bus: &mut dyn AddressBus,
        cycles: &mut u64,
    ) -> Result<(), CoreError> {
        let mut bytes = Vec::with_capacity(count as usize);
        for byte_offset in 0..count {
            bytes.push(bus.read_u8(source.wrapping_add(byte_offset), cycles)?);
        }
        self.flash
            .write(self.flash.base().wrapping_add(offset), &bytes)
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
                SYS_INFO_FLASH_DEV_INFO => words.push(u32::from(DEVINFO_VALUE)),
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
        // 设置 `RP2350_TRACE_BOOTROM=1` 可在排查固件调用 ROM API 时打印每次调用。
        // Set `RP2350_TRACE_BOOTROM=1` to log every ROM API call while debugging
        // firmware boot flows.
        if std::env::var_os("RP2350_TRACE_BOOTROM").is_some() {
            eprintln!(
                "[bootrom] id={:#06X} args={:#010X} {:#010X} {:#010X} {:#010X}",
                call.id.0, call.args[0], call.args[1], call.args[2], call.args[3]
            );
        }
        if call.id.0 == TRAP_LOOKUP {
            return Ok(TrapOutcome::value(self.table_lookup(call.args), 8));
        }
        if call.id.0 == TRAP_RV_LOOKUP {
            return Ok(TrapOutcome::value(Self::rv_table_lookup(call.args), 8));
        }
        if call.id.0 == TRAP_RV_DATA_LOOKUP {
            return Ok(TrapOutcome::value(Self::rv_data_lookup(call.args), 8));
        }
        // 未实现的 ROM 函数：指针有效但调用返回 0、无副作用。
        // Unimplemented ROM functions: the pointer is valid, the call returns 0
        // and has no side effects.
        if call.id.0 == TRAP_RV_ZERO_STUB {
            return Ok(TrapOutcome::void(8));
        }

        let [addr, arg1, arg2, arg3] = call.args;
        let outcome = match Trap::from_id(call.id.0) {
            Some(Trap::ConnectFlash | Trap::ExitXip | Trap::EnterCmdXip) => TrapOutcome::void(32),
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
    use rp2350_core::FlashRegion;

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
            Some(DEVINFO_PTR_OFFSET as u16)
        );
        // 第一级：表项指向的 32 位字面量给出信息字地址。
        // First level: the 32-bit literal named by the entry holds the word address.
        assert_eq!(
            u32::from_le_bytes(
                image[DEVINFO_PTR_OFFSET..DEVINFO_PTR_OFFSET + 4]
                    .try_into()
                    .unwrap()
            ),
            DEVINFO_OFFSET as u32
        );
        // 第二级：信息字本身为 CS0 = 16 MB（code 0xC 位于 bits 11:8）。
        // Second level: the info word itself is CS0 = 16 MB (code 0xC at bits 11:8).
        assert_eq!(
            u16::from_le_bytes(
                image[DEVINFO_OFFSET..DEVINFO_OFFSET + 2]
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
}

#[cfg(test)]
#[path = "tests_flash.rs"]
mod tests_flash;

#[cfg(test)]
#[path = "tests_riscv.rs"]
mod tests_riscv;
