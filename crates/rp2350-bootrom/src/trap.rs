//! Host-side trap dispatch for synthetic boot ROM stubs
//! 合成 Boot ROM 占位桩的宿主侧陷阱分发
//!
//! The guest sees `udf #id` at a stub address. [`TrapDispatcher`] recognises the
//! address before the next instruction step, invokes a chip-specific
//! [`TrapHandler`], writes the result back to `r0` and returns through `lr`.
//! The interface only depends on [`CpuCore`] and [`AddressBus`], so the CLI,
//! GDB server and GUI can share it.
//! 客户机在桩地址看到 `udf #id`。分发器在下一条指令步进前识别该地址，调用
//! 芯片相关的 [`TrapHandler`]，把结果写回 `r0` 并经 `lr` 返回。接口只依赖
//! [`CpuCore`] 与 [`AddressBus`]，CLI、GDB 服务器和 GUI 均可复用。

// 陷阱分发处理 32 位地址与周期计数，类型转换是仿真层的固有特性。
// Trap dispatch handles 32-bit addresses and cycle counts; the conversions are
// inherent to the emulation layer.
#![allow(clippy::cast_possible_truncation)]

use std::fmt;
use std::sync::Arc;

use rp2350_core::bus::AddressBus;
use rp2350_core::error::CoreError;
use rp2350_cpu::CpuCore;

/// Identifier carried by a `udf` stub / `udf` 占位桩携带的标识
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrapId(pub u16);

/// Register arguments captured from `r0..r3` when a trap fires
/// 陷阱触发时从 `r0..r3` 捕获的寄存器参数
#[derive(Debug, Clone, Copy)]
pub struct TrapCall {
    /// Trap identifier / 陷阱标识
    pub id: TrapId,
    /// Arguments in `r0`, `r1`, `r2`, `r3` order / 按 `r0`、`r1`、`r2`、`r3` 顺序的参数
    pub args: [u32; 4],
}

/// Outcome of a handled trap / 已处理陷阱的结果
#[derive(Debug, Clone, Copy)]
pub struct TrapOutcome {
    /// Value written back to `r0` / 写回 `r0` 的值
    pub result: u32,
    /// Approximate cycles consumed by the emulated function / 模拟函数消耗的近似周期
    pub cycles: u32,
    /// Set when the guest must stop instead of returning / 客户机必须停机而非返回时置位
    pub halt: Option<HaltReason>,
}

impl TrapOutcome {
    /// A function returning `0` / 返回 `0` 的函数
    #[must_use]
    pub const fn void(cycles: u32) -> Self {
        Self {
            result: 0,
            cycles,
            halt: None,
        }
    }

    /// A function returning `result` / 返回 `result` 的函数
    #[must_use]
    pub const fn value(result: u32, cycles: u32) -> Self {
        Self {
            result,
            cycles,
            halt: None,
        }
    }
}

/// Reason a trap stopped the guest / 陷阱使客户机停机的原因
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HaltReason {
    /// The guest called `rom_reboot` / 客户机调用了 `rom_reboot`
    Reboot {
        /// Reboot flags / 重启标志
        flags: u32,
        /// Delay in milliseconds / 延迟（毫秒）
        delay_ms: u32,
        /// Parameter 0 passed to the boot ROM / 传给 boot ROM 的参数 0
        p0: u32,
        /// Parameter 1 passed to the boot ROM / 传给 boot ROM 的参数 1
        p1: u32,
    },
}

impl fmt::Display for HaltReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reboot {
                flags,
                delay_ms,
                p0,
                p1,
            } => write!(
                f,
                "reboot requested (flags={flags:#010X}, delay={delay_ms} ms, p0={p0:#010X}, p1={p1:#010X})"
            ),
        }
    }
}

/// Chip-specific implementation behind the trap stubs
/// 陷阱占位桩背后的芯片相关实现
pub trait TrapHandler: Send + Sync {
    /// Execute one trap call / 执行一次陷阱调用
    ///
    /// # Errors
    /// Returns a bus error if an argument points at unmapped guest memory.
    /// 参数指向未映射的客户机内存时返回总线错误。
    fn invoke(
        &self,
        call: TrapCall,
        bus: &mut dyn AddressBus,
        cycles: &mut u64,
    ) -> Result<TrapOutcome, CoreError>;
}

/// Maps stub addresses to trap identifiers and applies their side effects
/// 将占位桩地址映射到陷阱标识并施加其副作用
pub struct TrapDispatcher {
    stubs: Vec<(u32, TrapId)>,
    handler: Arc<dyn TrapHandler>,
}

impl TrapDispatcher {
    /// Create a dispatcher backed by `handler` / 创建由 `handler` 支持的分发器
    #[must_use]
    pub fn new(handler: Arc<dyn TrapHandler>) -> Self {
        Self {
            stubs: Vec::new(),
            handler,
        }
    }

    /// Register the address of one stub / 注册一个占位桩的地址
    pub fn register(&mut self, pc: u32, id: TrapId) {
        self.stubs.push((pc, id));
    }

    /// Report whether `pc` is a registered stub / 报告 `pc` 是否为已注册占位桩
    #[must_use]
    pub fn handles(&self, pc: u32) -> bool {
        self.stubs.iter().any(|(addr, _)| *addr == pc)
    }

    /// Handle the trap at the core's current PC, if any
    /// 若核心当前 PC 位于占位桩，则处理该陷阱
    ///
    /// Returns `Ok(None)` when the PC is not a stub, leaving the caller free to
    /// execute the instruction normally.
    /// 当 PC 不是占位桩时返回 `Ok(None)`，调用方可以正常执行指令。
    ///
    /// # Errors
    /// Propagates bus errors from the chip handler.
    /// 传播芯片处理器产生的总线错误。
    pub fn dispatch(
        &self,
        core: &mut dyn CpuCore,
        bus: &mut dyn AddressBus,
    ) -> Result<Option<TrapOutcome>, CoreError> {
        let pc = core.pc();
        let Some(id) = self
            .stubs
            .iter()
            .find_map(|(addr, id)| (*addr == pc).then_some(*id))
        else {
            return Ok(None);
        };

        let call = TrapCall {
            id,
            args: [
                core.read_register(0),
                core.read_register(1),
                core.read_register(2),
                core.read_register(3),
            ],
        };
        let mut cycles = 0u64;
        let mut outcome = self.handler.invoke(call, bus, &mut cycles)?;
        outcome.cycles = outcome.cycles.saturating_add(cycles as u32);

        core.write_register(0, outcome.result);
        core.write_register(15, core.read_register(14) & !1);
        Ok(Some(outcome))
    }
}
