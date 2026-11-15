//! Optional memory watchpoints for debugging / 可选的内存监视点（调试用）
//!
//! Two independent channels live here.
//!
//! 1. An environment-variable diagnostic: `RP2350_WATCH=START-END` (hex) prints
//!    every SRAM write in `[START, END)` with the program counter of the
//!    instruction that performed it; `RP2350_WATCHV=VALUE` watches a written
//!    value instead. Both do nothing unless the variables are present.
//! 2. A runtime read watch armed by the GDB server: [`set_read_watch`] installs
//!    one `[start, end)` range, [`check_read`] records the latest hit, and
//!    [`take_read_hit`] consumes it once. When no range is armed the read path
//!    pays a single relaxed atomic load.
//!
//! 本模块包含两条互不影响的通道：
//!
//! 1. 环境变量诊断：`RP2350_WATCH=START-END`（十六进制）打印 `[START, END)`
//!    区间内每次 SRAM 写及其指令 PC；`RP2350_WATCHV=VALUE` 改为监视特定写值。
//!    未设置变量时完全空操作。
//! 2. GDB 服务器安装的运行期读监视：[`set_read_watch`] 安装一个 `[start, end)`
//!    区间，[`check_read`] 记录最近一次命中，[`take_read_hit`] 一次性取走。未安装
//!    区间时读路径只多一次 relaxed 原子 load。

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};

/// Fast gate: set once a runtime read watch has ever been armed
/// 快速门控：一旦安装过运行期读监视就置位
///
/// Checked before any thread-local access so a process that never armed a read
/// watch pays one relaxed atomic load per read. The gate is **sticky**: it is
/// never cleared, because the range itself lives in thread-local storage and a
/// different thread disarming its own watch must not make this thread miss
/// hits. / 在任何 thread-local 访问之前检查，因此从未安装读监视的进程每次读只付
/// 一次 relaxed 原子 load。门控**只置位不复位**：区间本身位于 thread-local，另一
/// 线程撤除自己的监视不得让本线程漏报。
static READ_ARMED: AtomicBool = AtomicBool::new(false);

/// Fast gate for the write channel / 写通道快速门控
///
/// Set when `RP2350_WATCH` / `RP2350_WATCHV` armed a write watch. Unset means
/// both write hooks are no-ops: [`set_pc`] skips its thread-local store and
/// [`check_write`] returns before touching thread-local storage, so a process
/// that never set the variables pays one relaxed atomic load per store.
/// 当 `RP2350_WATCH` / `RP2350_WATCHV` 安装了写监视时置位。未置位时两个写钩子
/// 都是空操作：[`set_pc`] 跳过 thread-local 写入，[`check_write`] 在访问
/// thread-local 之前返回，因此从未设置该变量的进程每次存储只付一次 relaxed
/// 原子 load。
static WRITE_ARMED: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// Watch range `[start, end)` parsed from the environment / 从环境解析的监视区间
    static RANGE: Cell<Option<(u32, u32)>> = const { Cell::new(None) };
    /// Watched write value parsed from the environment / 从环境解析的监视写值
    static VALUE: Cell<Option<u32>> = const { Cell::new(None) };
    /// Current program counter, refreshed once per instruction step
    /// 当前程序计数器，每条指令单步刷新一次
    static PC: Cell<u32> = const { Cell::new(0) };
    /// Runtime read-watch range installed by the debugger / 调试器安装的运行期读监视区间
    static READ_RANGE: Cell<Option<(u32, u32)>> = const { Cell::new(None) };
    /// Latest recorded read hit `(address, width)` / 最近一次读命中 `(地址, 宽度)`
    static READ_HIT: Cell<Option<(u32, u32)>> = const { Cell::new(None) };
    /// Host-side suppression flag for snapshot sampling / 宿主采样时的抑制标记
    static READ_SUSPEND: Cell<bool> = const { Cell::new(false) };
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
                    set_write_watch(Some((start, end)));
                }
            }
        }
    }
    if let Ok(spec) = std::env::var("RP2350_WATCHV") {
        if let Ok(value) = u32::from_str_radix(spec.trim().trim_start_matches("0x"), 16) {
            set_write_value(Some(value));
        }
    }
}

/// Arm or disarm the address-range write watch / 安装或撤除地址区间写监视
///
/// `Some((start, end))` reports every write whose address lies in
/// `[start, end)`; `None` disarms it. The environment variable
/// `RP2350_WATCH` is the command-line way to call this.
/// `Some((start, end))` 上报地址落在 `[start, end)` 的每次写；`None` 撤除。
/// 环境变量 `RP2350_WATCH` 是它在命令行上的入口。
pub fn set_write_watch(range: Option<(u32, u32)>) {
    RANGE.with(|r| r.set(range));
    refresh_write_gate();
}

/// Arm or disarm the written-value write watch / 安装或撤除写值监视
///
/// `Some(value)` reports every write of `value`; `None` disarms it. The
/// environment variable `RP2350_WATCHV` is the command-line way to call this.
/// `Some(value)` 上报每次写入该值的写；`None` 撤除。环境变量 `RP2350_WATCHV`
/// 是它在命令行上的入口。
pub fn set_write_value(value: Option<u32>) {
    VALUE.with(|v| v.set(value));
    refresh_write_gate();
}

/// Refresh the write gate after a channel changed / 通道变化后刷新写门控
fn refresh_write_gate() {
    let armed = RANGE.with(Cell::get).is_some() || VALUE.with(Cell::get).is_some();
    WRITE_ARMED.store(armed, Ordering::Relaxed);
}

/// Whether a write watch is armed / 是否已安装写监视
///
/// Callers that implement wide stores natively use this to decide between the
/// fast path and the per-byte path, which reproduces the diagnostic exactly.
/// 原生实现宽写入的调用方用它决定走快速路径还是逐字节路径，后者精确复现诊断输出。
#[must_use]
pub fn write_watch_armed() -> bool {
    WRITE_ARMED.load(Ordering::Relaxed)
}

/// Record the program counter of the instruction about to execute
/// 记录即将执行指令的程序计数器
pub fn set_pc(pc: u32) {
    // 未安装写监视时 PC 没有任何读者，跳过 thread-local 写入。
    // With no write watch armed nothing reads the PC, so skip the store.
    if !WRITE_ARMED.load(Ordering::Relaxed) {
        return;
    }
    PC.with(|p| p.set(pc));
}

/// Report a write that falls inside the watch range, or matches the watch value
/// 报告落入监视区间或匹配监视写值的写
pub fn check_write(addr: u32, value: u32) {
    if !WRITE_ARMED.load(Ordering::Relaxed) {
        return;
    }
    let hit = RANGE.with(|r| {
        r.get()
            .is_some_and(|(start, end)| addr >= start && addr < end)
    }) || VALUE.with(|v| v.get() == Some(value));
    if hit {
        let pc = PC.with(Cell::get);
        eprintln!("[watch] pc={pc:#010X} addr={addr:#010X} value={value:#010X}");
    }
}

/// Arm or disarm the runtime read watch / 安装或撤除运行期读监视
///
/// `Some((start, end))` watches reads whose address lies in `[start, end)`;
/// `None` disarms the watch and costs one relaxed atomic load per read again.
/// A pending hit is kept until [`take_read_hit`] consumes it.
/// `Some((start, end))` 监视地址落入 `[start, end)` 的读；`None` 撤除监视，读操作
/// 恢复为一次 relaxed 原子 load 的开销。未取走的命中会保留到 [`take_read_hit`]。
pub fn set_read_watch(range: Option<(u32, u32)>) {
    READ_RANGE.with(|r| r.set(range));
    if range.is_none() {
        // 撤除时丢弃未取走的命中，避免重新安装后上报陈旧访问。
        // Drop an unconsumed hit on disarm so a later re-arm cannot report it.
        READ_HIT.with(|hit| hit.set(None));
    }
    // 门控只置位：区间按线程存放，撤除本方监视不能影响其他线程的判定。
    // Publish the range before raising the gate; the sticky flag is never
    // cleared because the range lives per thread.
    if range.is_some() {
        READ_ARMED.store(true, Ordering::Relaxed);
    }
}

/// Record a read of `width` bytes at `addr` when it falls inside the armed range
/// 当地址落在已安装区间内时记录一次宽度为 `width` 字节的读
///
/// Only the latest hit is kept: a halfword or word read that the bus lowers to
/// several byte accesses still produces one recorded hit, because the wider
/// accessors call this exactly once. / 只保留最新一次命中：总线把半字/字读拆成
/// 多次字节访问时也只记录一次，因为宽访问器只调用本函数一次。
pub fn check_read(addr: u32, width: u32) {
    if !READ_ARMED.load(Ordering::Relaxed) {
        return;
    }
    if READ_SUSPEND.with(Cell::get) {
        return;
    }
    let inside = READ_RANGE.with(|r| {
        r.get()
            .is_some_and(|(start, end)| addr >= start && addr < end)
    });
    if inside {
        READ_HIT.with(|hit| hit.set(Some((addr, width))));
    }
}

/// Take the pending read hit, if any / 取走待处理的读命中
///
/// The hit is cleared, so every recorded access is reported exactly once.
/// 命中会被清除，因此每次记录的访问只上报一次。
#[must_use]
pub fn take_read_hit() -> Option<(u32, u32)> {
    if !READ_ARMED.load(Ordering::Relaxed) {
        return None;
    }
    READ_HIT.with(|hit| hit.replace(None))
}

/// Run `body` without recording read hits / 在禁止记录读命中的情况下运行 `body`
///
/// The debugger's own memory reads (watchpoint snapshots and GDB `m` packets)
/// must not trigger a read watch; they wrap their bus accesses in this helper.
/// Restores the previous flag on drop, including when `body` panics.
/// 调试器自身的内存读（监视点快照与 GDB `m` 包）不得触发读监视，它们把总线访问
/// 包在本辅助函数里。标志在 `Drop` 时恢复，`body` panic 也不例外。
pub fn without_read_watch<R>(body: impl FnOnce() -> R) -> R {
    struct Restore(Option<bool>);

    impl Drop for Restore {
        fn drop(&mut self) {
            if let Some(previous) = self.0 {
                READ_SUSPEND.with(|flag| flag.set(previous));
            }
        }
    }

    // 只有已安装读监视时才需要动抑制标记，避免给普通执行增加开销。
    // Only touch the flag when a read watch is armed, keeping plain execution free.
    let guard = if READ_ARMED.load(Ordering::Relaxed) {
        Restore(Some(READ_SUSPEND.with(|flag| flag.replace(true))))
    } else {
        Restore(None)
    };
    let result = body();
    drop(guard);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reset the runtime read watch state between tests
    /// 在测试之间重置运行期读监视状态
    fn reset() {
        set_read_watch(None);
        let _ = take_read_hit();
        READ_SUSPEND.with(|flag| flag.set(false));
    }

    /// Reset the environment-configured write channels between tests
    /// 在测试之间重置由环境变量配置的写通道
    fn reset_write() {
        set_write_watch(None);
        set_write_value(None);
    }

    #[test]
    fn write_gate_is_closed_until_a_channel_is_armed() {
        reset_write();
        assert!(
            !write_watch_armed(),
            "no channel armed means the gate is off"
        );
        set_write_watch(Some((0x2000_0000, 0x2000_0010)));
        assert!(write_watch_armed(), "an address range opens the gate");
        set_write_watch(None);
        assert!(!write_watch_armed(), "disarming closes it again");
        set_write_value(Some(0x1234_5678));
        assert!(write_watch_armed(), "a value watch opens the gate");
        reset_write();
        assert!(!write_watch_armed());
    }

    #[test]
    fn pc_is_only_recorded_while_a_write_watch_is_armed() {
        reset_write();
        // 门控关闭时 set_pc 不得写入 thread-local。
        // With the gate closed `set_pc` must not touch thread-local storage.
        PC.with(|p| p.set(0xDEAD_BEEF));
        set_pc(0x1111_2222);
        assert_eq!(PC.with(Cell::get), 0xDEAD_BEEF);
        set_write_watch(Some((0, 1)));
        set_pc(0x1111_2222);
        assert_eq!(PC.with(Cell::get), 0x1111_2222);
        reset_write();
    }

    #[test]
    fn reads_are_ignored_without_an_armed_watch() {
        reset();
        check_read(0x2000_0000, 4);
        assert_eq!(take_read_hit(), None);
    }

    #[test]
    fn armed_watch_records_reads_inside_the_range() {
        reset();
        set_read_watch(Some((0x2000_0000, 0x2000_0010)));
        check_read(0x2000_0004, 4);
        assert_eq!(take_read_hit(), Some((0x2000_0004, 4)));
        set_read_watch(None);
    }

    #[test]
    fn armed_watch_ignores_reads_outside_the_range() {
        reset();
        set_read_watch(Some((0x2000_0000, 0x2000_0010)));
        check_read(0x2000_0010, 1);
        check_read(0x1FFF_FFFF, 1);
        assert_eq!(take_read_hit(), None);
        set_read_watch(None);
    }

    #[test]
    fn take_read_hit_returns_each_hit_once() {
        reset();
        set_read_watch(Some((0x2000_0000, 0x2000_0010)));
        check_read(0x2000_0000, 2);
        assert_eq!(take_read_hit(), Some((0x2000_0000, 2)));
        assert_eq!(take_read_hit(), None);
        set_read_watch(None);
    }

    #[test]
    fn latest_hit_and_width_are_reported() {
        reset();
        set_read_watch(Some((0x2000_0000, 0x2000_0010)));
        check_read(0x2000_0000, 1);
        check_read(0x2000_0008, 4);
        assert_eq!(take_read_hit(), Some((0x2000_0008, 4)));
        set_read_watch(None);
    }

    #[test]
    fn suspended_reads_are_not_recorded() {
        reset();
        set_read_watch(Some((0x2000_0000, 0x2000_0010)));
        without_read_watch(|| check_read(0x2000_0004, 4));
        assert_eq!(take_read_hit(), None);
        check_read(0x2000_0004, 4);
        assert_eq!(take_read_hit(), Some((0x2000_0004, 4)));
        set_read_watch(None);
    }

    #[test]
    fn disarming_clears_pending_hits_from_reporting() {
        reset();
        set_read_watch(Some((0x2000_0000, 0x2000_0010)));
        check_read(0x2000_0004, 4);
        set_read_watch(None);
        // 撤除后 `take_read_hit` 返回 `None`，旧命中不再上报；门控保持置位只是
        // 让其他线程仍能自行判定各自区间。
        // After disarming `take_read_hit` returns `None`; the sticky gate only
        // lets other threads keep evaluating their own range.
        assert_eq!(take_read_hit(), None);
        check_read(0x2000_0004, 4);
        assert_eq!(take_read_hit(), None);
    }
}
