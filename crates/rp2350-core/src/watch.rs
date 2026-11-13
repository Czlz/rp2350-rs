//! Optional memory-write watchpoint for debugging / 可选的写监视点（调试用）
//!
//! Enabled by the `RP2350_WATCH=START-END` environment variable (hex). When
//! set, every SRAM write whose address falls in `[START, END)` is printed
//! together with the program counter of the instruction performing it. This is
//! a throwaway diagnostic for locating a store that corrupts guest memory; it
//! does nothing unless the variable is present.
//! 由环境变量 `RP2350_WATCH=START-END`（十六进制）启用。启用后，地址落入
//! `[START, END)` 区间的每次 SRAM 写都会连同执行它的指令 PC 一起打印。这是
//! 定位污染客户机内存的存储指令的一次性诊断；未设置变量时完全空操作。

use std::cell::Cell;

thread_local! {
    /// Watch range `[start, end)` parsed from the environment / 从环境解析的监视区间
    static RANGE: Cell<Option<(u32, u32)>> = const { Cell::new(None) };
    /// Watched write value parsed from the environment / 从环境解析的监视写值
    static VALUE: Cell<Option<u32>> = const { Cell::new(None) };
    /// Current program counter, refreshed once per instruction step
    /// 当前程序计数器，每条指令单步刷新一次
    static PC: Cell<u32> = const { Cell::new(0) };
}

/// Install the watch range from `RP2350_WATCH`, if present
/// 若存在则从 `RP2350_WATCH` 安装监视区间
///
/// `RP2350_WATCH=START-END` watches an address range. `RP2350_WATCHV=VALUE`
/// watches for a specific written value instead.
/// `RP2350_WATCH=START-END` 监视地址区间；`RP2350_WATCHV=VALUE` 改为监视特定写值。
pub fn init() {
    if let Ok(spec) = std::env::var("RP2350_WATCH") {
        if let Some((a, b)) = spec.split_once('-') {
            let parse = |s: &str| u32::from_str_radix(s.trim().trim_start_matches("0x"), 16);
            if let (Ok(start), Ok(end)) = (parse(a), parse(b)) {
                if start < end {
                    RANGE.with(|r| r.set(Some((start, end))));
                }
            }
        }
    }
    if let Ok(spec) = std::env::var("RP2350_WATCHV") {
        if let Ok(value) = u32::from_str_radix(spec.trim().trim_start_matches("0x"), 16) {
            VALUE.with(|v| v.set(Some(value)));
        }
    }
}

/// Record the program counter of the instruction about to execute
/// 记录即将执行指令的程序计数器
pub fn set_pc(pc: u32) {
    PC.with(|p| p.set(pc));
}

/// Report a write that falls inside the watch range, or matches the watch value
/// 报告落入监视区间或匹配监视写值的写
pub fn check_write(addr: u32, value: u32) {
    let hit = RANGE.with(|r| {
        r.get()
            .is_some_and(|(start, end)| addr >= start && addr < end)
    }) || VALUE.with(|v| v.get() == Some(value));
    if hit {
        let pc = PC.with(Cell::get);
        eprintln!("[watch] pc={pc:#010X} addr={addr:#010X} value={value:#010X}");
    }
}
