//! Guest traps and PMP-checked memory helpers / 客户机陷阱与经 PMP 检查的内存辅助
//!
//! The execution core builds a [`Trap`] for every guest-visible fault, and the
//! load/store helpers here apply the PMP check before touching the bus. Keeping
//! them out of `execute.rs` keeps the instruction decoder within the project's
//! per-file size guideline.
//! 执行核心为每个客户机可见故障构造 [`Trap`]；本模块的载入/存储辅助在访问总线前
//! 执行 PMP 检查。把它们从 `execute.rs` 拆出，使指令译码器符合项目单文件行数规范。

use crate::csr::CsrFile;
use crate::pmp::PmpAccess;
use rp2350_core::bus::AddressBus;
use rp2350_core::error::CoreError;

/// Internal trap raised while executing one instruction / 单条指令执行期间产生的内部陷阱
pub(crate) struct Trap {
    /// `mcause` value / 异常原因
    pub(crate) cause: u32,
    /// `mtval` value / 陷阱值
    pub(crate) tval: u32,
    /// Whether the access is retryable instead of trapping / 该访问是否应重试而非触发陷阱
    ///
    /// A device that blocks (an empty FIFO read) reports a bus stall. The guest
    /// must never observe an exception for it, so the flag makes the core put
    /// itself to sleep with `PC` unchanged and replay the same instruction later.
    /// 阻塞设备（空 FIFO 读）会报告总线 stall。客户机绝不能因此看到异常，因此该标志
    /// 让核心保持 `PC` 不变并进入睡眠，稍后重放同一条指令。
    pub(crate) retry: bool,
}

impl Trap {
    /// Build an illegal-instruction trap / 构造非法指令陷阱
    pub(crate) fn illegal(instr: u32) -> Self {
        Self {
            cause: 2,
            tval: instr,
            retry: false,
        }
    }

    /// Build an illegal-CSR trap: `mtval` carries the CSR address
    /// 构造非法 CSR 陷阱：`mtval` 携带 CSR 地址
    ///
    /// Both an unimplemented CSR and an illegal access (write to a read-only CSR)
    /// raise cause 2. The faulting CSR address is far more useful in `mtval` than
    /// the raw encoding, because the instruction is always a `csr*` form.
    /// 未实现的 CSR 与非法访问（写只读 CSR）都触发 cause 2。出错指令必然是
    /// `csr*` 形式，因此把 CSR 地址放进 `mtval` 比原始编码更有诊断价值。
    pub(crate) fn illegal_csr(csr: u32) -> Self {
        Self {
            cause: 2,
            tval: csr,
            retry: false,
        }
    }

    /// Build a retryable load trap / 构造可重试的加载陷阱
    fn retry(addr: u32) -> Self {
        Self {
            cause: 5,
            tval: addr,
            retry: true,
        }
    }

    /// Build a load access fault / 构造加载访问错误
    pub(crate) fn load_fault(addr: u32, error: &CoreError) -> Self {
        if error.is_stall() {
            Self::retry(addr)
        } else {
            Self {
                cause: 5,
                tval: addr,
                retry: false,
            }
        }
    }

    /// Build a store access fault / 构造存储访问错误
    fn store_fault(addr: u32) -> Self {
        Self {
            cause: 7,
            tval: addr,
            retry: false,
        }
    }

    /// Build an instruction access fault (PMP deny) / 构造取指访问错误（PMP 拒绝）
    pub(crate) fn instruction_access_fault(addr: u32) -> Self {
        Self {
            cause: 1,
            tval: addr,
            retry: false,
        }
    }

    /// Build a load access fault raised by PMP / 构造由 PMP 触发的加载访问错误
    fn load_access_fault(addr: u32) -> Self {
        Self {
            cause: 5,
            tval: addr,
            retry: false,
        }
    }

    /// Build a store access fault raised by PMP / 构造由 PMP 触发的存储访问错误
    fn store_access_fault(addr: u32) -> Self {
        Self {
            cause: 7,
            tval: addr,
            retry: false,
        }
    }
}

/// Read a load value by `funct3`, after the PMP check
/// 执行 PMP 检查后按 `funct3` 读取加载值
pub(crate) fn load_value(
    csr: &CsrFile,
    bus: &mut dyn AddressBus,
    addr: u32,
    funct3: u32,
) -> Result<u32, Trap> {
    let size = match funct3 {
        0 | 4 => 1,
        1 | 5 => 2,
        2 => 4,
        _ => return Err(Trap::illegal(funct3)),
    };
    if !csr.pmp_permits(addr, size, PmpAccess::Load, csr.data_access_is_machine()) {
        return Err(Trap::load_access_fault(addr));
    }
    let mut cycles = 0u64;
    match funct3 {
        // LB/LH sign-extend to XLEN; LBU/LHU below zero-extend.
        // LB/LH 符号扩展到 XLEN；下面的 LBU/LHU 做零扩展。
        0 => bus
            .read_u8(addr, &mut cycles)
            .map(|v| v as i8 as u32)
            .map_err(|error| Trap::load_fault(addr, &error)),
        1 => bus
            .read_u16(addr, &mut cycles)
            .map(|v| v as i16 as u32)
            .map_err(|error| Trap::load_fault(addr, &error)),
        2 => bus
            .read_u32(addr, &mut cycles)
            .map_err(|error| Trap::load_fault(addr, &error)),
        4 => bus
            .read_u8(addr, &mut cycles)
            .map(u32::from)
            .map_err(|error| Trap::load_fault(addr, &error)),
        5 => bus
            .read_u16(addr, &mut cycles)
            .map(u32::from)
            .map_err(|error| Trap::load_fault(addr, &error)),
        _ => Err(Trap::illegal(funct3)),
    }
}

/// Write a store value by `funct3`, after the PMP check
/// 执行 PMP 检查后按 `funct3` 写入存储值
pub(crate) fn store_value(
    csr: &CsrFile,
    bus: &mut dyn AddressBus,
    addr: u32,
    funct3: u32,
    value: u32,
) -> Result<(), Trap> {
    let size = match funct3 {
        0 => 1,
        1 => 2,
        2 => 4,
        _ => return Err(Trap::illegal(funct3)),
    };
    if !csr.pmp_permits(addr, size, PmpAccess::Store, csr.data_access_is_machine()) {
        return Err(Trap::store_access_fault(addr));
    }
    let mut cycles = 0u64;
    let fault = Trap::store_fault(addr);
    match funct3 {
        0 => bus
            .write_u8(addr, value as u8, &mut cycles)
            .map_err(|_| fault),
        1 => bus
            .write_u16(addr, value as u16, &mut cycles)
            .map_err(|_| fault),
        2 => bus.write_u32(addr, value, &mut cycles).map_err(|_| fault),
        _ => Err(Trap::illegal(funct3)),
    }
}

/// Load a 32-bit word (atomics, Zcmp pop), after the PMP check
/// 执行 PMP 检查后加载 32 位字（原子操作、Zcmp pop）
pub(crate) fn load_word(csr: &CsrFile, bus: &mut dyn AddressBus, addr: u32) -> Result<u32, Trap> {
    if !csr.pmp_permits(addr, 4, PmpAccess::Load, csr.data_access_is_machine()) {
        return Err(Trap::load_access_fault(addr));
    }
    let mut cycles = 0u64;
    bus.read_u32(addr, &mut cycles)
        .map_err(|error| Trap::load_fault(addr, &error))
}

/// Store a 32-bit word (atomics, Zcmp push), after the PMP check
/// 执行 PMP 检查后存储 32 位字（原子操作、Zcmp push）
pub(crate) fn store_word(
    csr: &CsrFile,
    bus: &mut dyn AddressBus,
    addr: u32,
    value: u32,
) -> Result<(), Trap> {
    // An SC.W with no valid reservation performs no bus write, so only the
    // executed store is checked here.
    // 无有效保留区的 SC.W 不产生总线写，因此这里只检查实际执行的存储。
    if !csr.pmp_permits(addr, 4, PmpAccess::Store, csr.data_access_is_machine()) {
        return Err(Trap::store_access_fault(addr));
    }
    let mut cycles = 0u64;
    bus.write_u32(addr, value, &mut cycles)
        .map_err(|_| Trap::store_fault(addr))
}
