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

/// Small flat memory used as the address bus for instruction tests
/// 指令测试中用作地址总线的小型平坦内存
pub(super) struct TestBus {
    /// Backing bytes, addresses 0x0000_0000..=0x0000_0FFF
    /// 后备字节，地址范围 0x0000_0000..=0x0000_0FFF
    pub(super) memory: Vec<u8>,
}

impl TestBus {
    /// Create a zeroed test bus covering 4 KiB
    /// 创建覆盖 4 KiB 的零初始化测试总线
    pub(super) fn new() -> Self {
        Self {
            memory: vec![0; 0x1000],
        }
    }

    fn offset(&self, addr: u32, len: u32) -> BusResult<usize> {
        let end = addr as usize + len as usize;
        if end > self.memory.len() {
            Err(CoreError::BusError {
                addr,
                kind: BusErrorKind::NoDevice,
            })
        } else {
            Ok(addr as usize)
        }
    }

    /// Write a byte directly, bypassing the bus trait
    /// 直接写入一个字节，绕过总线 trait
    pub(super) fn poke_u8(&mut self, addr: u32, value: u8) {
        self.memory[addr as usize] = value;
    }

    /// Write a halfword directly, bypassing the bus trait
    /// 直接写入一个半字，绕过总线 trait
    pub(super) fn poke_u16(&mut self, addr: u32, value: u16) {
        let offset = addr as usize;
        self.memory[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    /// Write a word directly, bypassing the bus trait
    /// 直接写入一个字，绕过总线 trait
    pub(super) fn poke_u32(&mut self, addr: u32, value: u32) {
        let offset = addr as usize;
        self.memory[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    /// Read a word directly, bypassing the bus trait
    /// 直接读取一个字，绕过总线 trait
    pub(super) fn peek_u32(&self, addr: u32) -> u32 {
        let offset = addr as usize;
        u32::from_le_bytes(self.memory[offset..offset + 4].try_into().unwrap())
    }

    /// Read a byte directly, bypassing the bus trait
    /// 直接读取一个字节，绕过总线 trait
    pub(super) fn peek_u8(&self, addr: u32) -> u8 {
        self.memory[addr as usize]
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
