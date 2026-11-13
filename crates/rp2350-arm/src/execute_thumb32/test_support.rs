//! Shared test harness for Thumb-32 instruction tests
//! Thumb-32 指令测试的共享测试装置

use rp2350_core::bus::{AddressBus, BusResult, MemoryRegion};
use rp2350_core::error::{BusErrorKind, CoreError};

use crate::conditions::ITBlockState;
use crate::execute_thumb16::StepResult;
use crate::execute_thumb32::execute_thumb32;
use crate::nvic::Nvic;
use crate::registers::CortexM33Registers;
use crate::systick::SysTick;

/// SIO base address mirrored by [`TestBus`] / [`TestBus`] 镜像的 SIO 基地址
const SIO_BASE: u32 = 0xD000_0000;
/// Size of the mirrored SIO window / 镜像 SIO 窗口大小
const SIO_WINDOW: u32 = 0x1000;
/// Offset of the mirrored SIO window in [`TestBus::memory`]
/// [`TestBus::memory`] 中镜像 SIO 窗口的偏移
const SIO_MIRROR_OFFSET: usize = 0x1000;

/// Small flat memory used as the address bus for instruction tests
/// 指令测试中用作地址总线的小型平坦内存
pub(super) struct TestBus {
    /// Backing bytes for addresses 0x0000_0000..=0x0000_0FFF followed by a
    /// mirror of the SIO window at 0xD000_0000..=0xD000_0FFF
    /// 地址 0x0000_0000..=0x0000_0FFF 的后备字节，其后是
    /// 0xD000_0000..=0xD000_0FFF SIO 窗口的镜像
    pub(super) memory: Vec<u8>,
}

impl TestBus {
    /// Create a zeroed test bus covering 4 KiB plus the SIO mirror
    /// 创建覆盖 4 KiB 及 SIO 镜像的零初始化测试总线
    pub(super) fn new() -> Self {
        Self {
            memory: vec![0; SIO_MIRROR_OFFSET + SIO_WINDOW as usize],
        }
    }

    /// Translate a bus address into an offset in the backing memory
    /// 将总线地址转换为后备内存偏移
    fn flat_offset(addr: u32) -> usize {
        if (SIO_BASE..SIO_BASE + SIO_WINDOW).contains(&addr) {
            SIO_MIRROR_OFFSET + (addr - SIO_BASE) as usize
        } else {
            addr as usize
        }
    }

    fn offset(&self, addr: u32, len: u32) -> BusResult<usize> {
        let offset = Self::flat_offset(addr);
        let end = offset + len as usize;
        if end > self.memory.len() {
            Err(CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            })
        } else {
            Ok(offset)
        }
    }

    /// Write a byte directly, bypassing the bus trait
    /// 直接写入一个字节，绕过总线 trait
    pub(super) fn poke_u8(&mut self, addr: u32, value: u8) {
        self.memory[Self::flat_offset(addr)] = value;
    }

    /// Write a halfword directly, bypassing the bus trait
    /// 直接写入一个半字，绕过总线 trait
    pub(super) fn poke_u16(&mut self, addr: u32, value: u16) {
        let offset = Self::flat_offset(addr);
        self.memory[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    /// Write a word directly, bypassing the bus trait
    /// 直接写入一个字，绕过总线 trait
    pub(super) fn poke_u32(&mut self, addr: u32, value: u32) {
        let offset = Self::flat_offset(addr);
        self.memory[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    /// Read a word directly, bypassing the bus trait
    /// 直接读取一个字，绕过总线 trait
    pub(super) fn peek_u32(&self, addr: u32) -> u32 {
        let offset = Self::flat_offset(addr);
        u32::from_le_bytes(self.memory[offset..offset + 4].try_into().unwrap())
    }

    /// Read a byte directly, bypassing the bus trait
    /// 直接读取一个字节，绕过总线 trait
    pub(super) fn peek_u8(&self, addr: u32) -> u8 {
        self.memory[Self::flat_offset(addr)]
    }
}

impl AddressBus for TestBus {
    fn register_region(&mut self, _region: Box<dyn MemoryRegion>) -> BusResult<()> {
        Ok(())
    }

    fn unregister_region(&mut self, _name: &str) {}

    fn find_region_mut(&mut self, _addr: u32) -> Option<&mut dyn MemoryRegion> {
        None
    }

    fn read_u8(&mut self, addr: u32, _cycles: &mut u64) -> BusResult<u8> {
        Ok(self.memory[self.offset(addr, 1)?])
    }

    fn write_u8(&mut self, addr: u32, value: u8, _cycles: &mut u64) -> BusResult<()> {
        let offset = self.offset(addr, 1)?;
        self.memory[offset] = value;
        Ok(())
    }

    fn read_u16(&mut self, addr: u32, _cycles: &mut u64) -> BusResult<u16> {
        let offset = self.offset(addr, 2)?;
        Ok(u16::from_le_bytes(
            self.memory[offset..offset + 2].try_into().unwrap(),
        ))
    }

    fn write_u16(&mut self, addr: u32, value: u16, _cycles: &mut u64) -> BusResult<()> {
        let offset = self.offset(addr, 2)?;
        self.memory[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    fn read_u32(&mut self, addr: u32, _cycles: &mut u64) -> BusResult<u32> {
        let offset = self.offset(addr, 4)?;
        Ok(u32::from_le_bytes(
            self.memory[offset..offset + 4].try_into().unwrap(),
        ))
    }

    fn write_u32(&mut self, addr: u32, value: u32, _cycles: &mut u64) -> BusResult<()> {
        let offset = self.offset(addr, 4)?;
        self.memory[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }
}

/// Execute one instruction on a zeroed bus and return its result
/// 在零初始化总线上执行一条指令并返回结果
pub(super) fn execute(instr: u32, regs: &mut CortexM33Registers) -> StepResult {
    let mut bus = TestBus::new();
    run(instr, regs, &mut bus)
}

/// Execute one instruction on a caller-provided bus
/// 在调用者提供的总线上执行一条指令
pub(super) fn run(instr: u32, regs: &mut CortexM33Registers, bus: &mut TestBus) -> StepResult {
    let mut nvic = Nvic::new();
    let mut systick = SysTick::new();
    let mut it = ITBlockState::new();
    let mut cycles = 0u64;
    execute_thumb32(
        instr,
        regs,
        bus,
        &mut nvic,
        &mut systick,
        &mut it,
        &mut cycles,
    )
    .expect("test instruction must not fault")
}
