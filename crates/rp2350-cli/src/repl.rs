//! `repl` subcommand: drive the USB CDC raw REPL of a booted machine
//! `repl` 子命令：驱动已启动机器的 USB CDC raw REPL
//!
//! Stock `MicroPython` RP2350 firmware exposes its REPL only over USB CDC, so the
//! host must first let the guest boot, enumerate the emulated CDC device and
//! then speak the raw REPL protocol over the bulk endpoints discovered by
//! [`UsbHostBridge`]. / 原生 `MicroPython` RP2350 固件只在 USB CDC 上暴露 REPL，
//! 因此宿主必须先让客户机启动、枚举模拟 CDC 设备，再通过
//! [`UsbHostBridge`] 发现的批量端点使用 raw REPL 协议。

use std::fmt;
use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};
use rp2350_usb_host::repl::BANNER_LINE;
use rp2350_usb_host::{
    CdcChannel, CdcEndpoints, CdcLink, HostError, HostProgress, RawReplSession, UsbHostBridge,
};

use crate::run::{ArchArg, Machine, MachineOptions, fault_tail_lines};

/// Default total instruction budget for `repl` / `repl` 的默认总指令预算
pub const DEFAULT_MAX_CYCLES: u64 = 300_000_000;

/// Default budget allowed for USB enumeration / USB 枚举的默认预算
pub const DEFAULT_ENUM_CYCLES: u64 = 100_000_000;

/// Options for a `repl` invocation / `repl` 调用选项
#[derive(Debug)]
pub struct ReplOptions {
    /// Firmware file (UF2 or ELF) / 固件文件（UF2 或 ELF）
    pub file: PathBuf,
    /// Architecture override; UF2 family auto-detection is used when absent
    /// 架构覆盖；缺省时使用 UF2 family 自动识别
    pub arch: Option<ArchArg>,
    /// Optional Python source executed after entering raw REPL
    /// 进入 raw REPL 后可选执行的 Python 源码
    pub exec: Option<String>,
    /// Total instruction budget / 总指令预算
    pub max_cycles: u64,
    /// Budget allowed before USB enumeration completes / USB 枚举完成前的预算
    pub enum_cycles: u64,
    /// Perform the Ctrl-D soft reset when entering raw REPL
    /// 进入 raw REPL 时执行 Ctrl-D 软复位
    pub soft_reset: bool,
    /// Halt on undefined instructions instead of skipping them
    /// 遇到未定义指令时停机而不是跳过
    pub strict_decode: bool,
}

/// An open USB CDC raw REPL session borrowing the machine
/// 借用机器的已打开 USB CDC raw REPL 会话
// A boxed cycle runner keeps the session's link type nameable while the closure
// borrows the machine. / 装箱的周期闭包让会话链路类型可命名，同时闭包借用机器。
#[allow(clippy::type_complexity)]
pub struct OpenSession<'a> {
    /// Discovered CDC bulk endpoints / 发现的 CDC 批量端点
    pub endpoints: CdcEndpoints,
    /// Raw REPL session over the CDC byte link / CDC 字节链路上的 raw REPL 会话
    pub session: RawReplSession<CdcLink<Box<dyn FnMut(u64) + 'a>>>,
}

/// Why USB enumeration did not reach `Configured`
/// USB 枚举未能到达 `Configured` 的原因
#[derive(Debug)]
pub enum EnumerationError {
    /// The host bridge rejected a control transfer / 主机桥拒绝某次控制传输
    Failed {
        /// Bridge error / 主机桥错误
        error: HostError,
        /// Device descriptor summary, if one was parsed / 已解析的设备描述符摘要
        device: String,
        /// Assigned device address / 已分配的设备地址
        address: u8,
        /// Host bridge stage and transfer snapshot / 主机桥阶段与传输快照
        progress: String,
    },
    /// The device disappeared during enumeration / 枚举期间设备消失
    Detached {
        /// Executed steps / 已执行步数
        steps: u64,
        /// Last program counter / 最后程序计数器
        pc: u32,
    },
    /// The enumeration budget ran out / 枚举预算耗尽
    Timeout {
        /// Executed steps / 已执行步数
        steps: u64,
        /// Last program counter / 最后程序计数器
        pc: u32,
        /// Description of the last poll result / 最后一次轮询结果描述
        last: String,
        /// Machine instruction budget / 机器指令预算
        budget: u64,
        /// Machine halt reason, when it stopped for another reason
        /// 机器因其他原因停机时的原因
        halt: Option<String>,
        /// Host bridge stage and transfer snapshot / 主机桥阶段与传输快照
        progress: String,
    },
    /// The guest stopped before USB enumeration completed
    /// 客户机在 USB 枚举完成前停机
    Halted {
        /// Executed steps / 已执行步数
        steps: u64,
        /// Last program counter / 最后程序计数器
        pc: u32,
        /// Halt reason / 停机原因
        reason: String,
    },
}

impl fmt::Display for EnumerationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failed {
                error,
                device,
                address,
                progress,
            } => write!(
                f,
                "USB enumeration failed / USB 枚举失败: {error:?}; device={device}; \
                 address={address}; {progress}"
            ),
            Self::Detached { steps, pc } => write!(
                f,
                "USB device detached during enumeration / USB 枚举期间设备消失: \
                 steps={steps}, PC={pc:#010X}"
            ),
            Self::Timeout {
                steps,
                pc,
                last,
                budget,
                halt,
                progress,
            } => {
                write!(
                    f,
                    "USB enumeration timed out / USB 枚举超时: steps={steps}, PC={pc:#010X}, \
                     last poll: {last}, machine budget={budget} cycles, {progress}"
                )?;
                if let Some(halt) = halt {
                    write!(f, ", halt: {halt}")?;
                }
                Ok(())
            }
            Self::Halted { steps, pc, reason } => write!(
                f,
                "machine halted during enumeration / 枚举期间机器停机: {reason} \
                 (steps={steps}, PC={pc:#010X})"
            ),
        }
    }
}

impl std::error::Error for EnumerationError {}

/// Build a machine and enumerate its USB CDC function
/// 构建机器并枚举其 USB CDC 功能
///
/// The guest keeps running between [`UsbHostBridge::poll`] calls: `now` is the
/// machine's simulated cycle count, and each `Wait` hint advances the CPU to the
/// requested cycle (capped by the enumeration deadline). / 在
/// [`UsbHostBridge::poll`] 调用之间客户机持续运行：`now` 是机器模拟周期数，
/// 每个 `Wait` 提示都会把 CPU 推进到请求周期（受枚举期限限制）。
///
/// # Errors
/// Returns [`EnumerationError`] when the guest halts, detaches, fails a control
/// transfer, or exhausts the enumeration/machine budget. / 客户机停机、设备
/// 消失、控制传输失败或枚举/机器预算耗尽时返回 [`EnumerationError`]。
pub fn open_session<'a>(
    machine: &'a mut Machine,
    enum_cycles: u64,
) -> Result<OpenSession<'a>, EnumerationError> {
    let start = machine.now_cycles();
    let deadline = start.saturating_add(enum_cycles);
    let mut bridge = UsbHostBridge::new(machine.usb().clone());
    let mut last = String::from("not polled");
    let endpoints = loop {
        if let Some(reason) = machine.halt_reason() {
            return Err(EnumerationError::Halted {
                steps: machine.steps(),
                pc: machine.pc(),
                reason: reason.to_string(),
            });
        }
        let now = machine.now_cycles();
        if machine.budget_exhausted() || now >= deadline {
            return Err(EnumerationError::Timeout {
                steps: machine.steps(),
                pc: machine.pc(),
                last,
                budget: machine.budget(),
                halt: machine.halt_reason().map(str::to_owned),
                progress: bridge.progress_line(),
            });
        }
        match bridge.poll(now) {
            HostProgress::Configured(endpoints) => break endpoints,
            HostProgress::Failed(error) => {
                return Err(EnumerationError::Failed {
                    error,
                    device: describe_device(&bridge),
                    address: bridge.address(),
                    progress: bridge.progress_line(),
                });
            }
            HostProgress::Detached => {
                return Err(EnumerationError::Detached {
                    steps: machine.steps(),
                    pc: machine.pc(),
                });
            }
            HostProgress::Wait(next) => {
                last = format!("waiting until cycle {next}");
                machine.run_cycles(advance_to(now, next, deadline));
            }
        }
    };

    let channel = CdcChannel::new(machine.usb().clone(), endpoints);
    let runner: Box<dyn FnMut(u64) + 'a> = Box::new(move |cycles| machine.run_cycles(cycles));
    let link = CdcLink::new(channel, runner);
    Ok(OpenSession {
        endpoints,
        session: RawReplSession::new(link),
    })
}

/// Cycles to run before polling again, capped by the enumeration deadline
/// 再次轮询前应运行的周期数，受枚举期限限制
fn advance_to(now: u64, wait_until: u64, deadline: u64) -> u64 {
    wait_until
        .max(now.saturating_add(1))
        .min(deadline)
        .saturating_sub(now)
}

/// Summarise the parsed device descriptor / 汇总已解析的设备描述符
fn describe_device(bridge: &UsbHostBridge) -> String {
    bridge.device_info().map_or_else(
        || String::from("no device descriptor"),
        |info| format!("VID=0x{:04X} PID=0x{:04X}", info.vendor_id, info.product_id),
    )
}

/// Open the raw REPL, optionally execute source, then leave raw mode
/// 打开 raw REPL，可选执行源码，然后退出 raw 模式
///
/// # Errors
/// Returns an error when the firmware cannot be loaded, USB enumeration fails,
/// the raw REPL protocol times out, or executed source writes to stderr.
/// 固件无法加载、USB 枚举失败、raw REPL 协议超时或执行源码写入标准错误时返回错误。
pub fn run(options: &ReplOptions) -> Result<()> {
    let bytes = std::fs::read(&options.file).with_context(|| {
        format!(
            "cannot read firmware / 无法读取固件: {}",
            options.file.display()
        )
    })?;
    let machine_options = MachineOptions {
        arch: options.arch,
        max_cycles: options.max_cycles,
        strict_decode: options.strict_decode,
    };
    let mut machine = Machine::new(&bytes, &machine_options)?;
    println!("[repl] {}", machine.image_description());
    println!("[repl] core: {}", machine.core_description());

    let source = options.exec.clone();
    let result = (|| -> Result<()> {
        let mut open =
            open_session(&mut machine, options.enum_cycles).map_err(|error| anyhow!(error))?;
        println!(
            "[repl] USB CDC configured: IN=0x{:02X}/{} OUT=0x{:02X}/{} interface={}",
            open.endpoints.in_address,
            open.endpoints.in_max_packet,
            open.endpoints.out_address,
            open.endpoints.out_max_packet,
            open.endpoints.interface
        );
        open.session
            .enter_raw_repl(options.soft_reset)
            .map_err(|error| anyhow!("enter raw REPL failed / 进入 raw REPL 失败: {error:?}"))?;
        println!(
            "[repl] banner: {}",
            String::from_utf8_lossy(BANNER_LINE).trim_end()
        );
        if let Some(source) = source {
            let output = open.session.exec_raw(source.as_bytes()).map_err(|error| {
                anyhow!("raw REPL execution failed / raw REPL 执行失败: {error:?}")
            })?;
            write_output(&output.stdout, &output.stderr)?;
            if !output.stderr.is_empty() {
                bail!("script reported an error / 脚本报告错误");
            }
        }
        open.session
            .exit_raw_repl()
            .map_err(|error| anyhow!("exit raw REPL failed / 退出 raw REPL 失败: {error:?}"))?;
        Ok(())
    })();
    machine.finalize();

    let state = machine_state(&machine);
    match result {
        Ok(()) => {
            println!("[repl] {state}");
            Ok(())
        }
        Err(error) => {
            // 客户机故障往往只留下一个最终 PC；附上停机前的指令轨迹，
            // 才能判断是哪条分支或异常返回跑飞。
            // A guest fault often leaves only a final PC; attach the recent
            // instruction history so the offending branch is identifiable.
            for line in fault_tail_lines(&mut machine) {
                eprintln!("[repl] {line}");
            }
            // 崩溃现场的对象与跳转目标转储：间接调用跳到栈地址通常是对象
            // 的函数指针字段被污染，或该地址本身未映射。
            // Dump the faulting object and branch target: an indirect call that
            // lands on a stack address usually means a polluted vtable/function
            // pointer, or an unmapped destination.
            let obj = machine.register(4);
            let target = machine.register(3);
            if obj & 0xFFF0_0000 == 0x2000_0000 {
                eprintln!("[repl] object@R4={obj:#010X}:");
                for (a, v) in machine.dump_memory(obj, 16) {
                    eprintln!("[repl]   {a:#010X}: {v:#010X}");
                }
            }
            if target & 0xFFF0_0000 == 0x2000_0000 {
                eprintln!("[repl] target@R3={target:#010X}:");
                for (a, v) in machine.dump_memory(target, 8) {
                    eprintln!("[repl]   {a:#010X}: {v:#010X}");
                }
            }
            // USB 枚举卡住时，中断是否送达客户机是第一手证据。
            // Whether IRQs reached the guest is the first thing to check on a
            // stalled USB enumeration.
            for line in machine.irqs() {
                eprintln!("[repl] {line}");
            }
            for line in machine.traps() {
                eprintln!("[repl] {line}");
            }
            // 控制器中断聚合状态区分「客户机未布防端点」与「中断未送达」。
            // The aggregated controller interrupt state distinguishes "guest
            // never armed EP0" from "IRQ was not delivered".
            let usb = machine.usb().clone();
            usb.with_core(|core| {
                eprintln!(
                    "[repl] usb INTR={:#010X} INTE={:#010X} INTS={:#010X} SIE={:#010X} BUFF={:#010X}",
                    core.read_reg(rp2350_usb::regs::INTR),
                    core.read_reg(rp2350_usb::regs::INTE),
                    core.read_reg(rp2350_usb::regs::INTS),
                    core.read_reg(rp2350_usb::regs::SIE_STATUS),
                    core.read_reg(rp2350_usb::regs::BUFF_STATUS),
                );
            });
            Err(error.context(state))
        }
    }
}

/// Print captured stdout and stderr with explicit labels
/// 带明确标签地打印捕获的标准输出与标准错误
fn write_output(stdout: &[u8], stderr: &[u8]) -> Result<()> {
    let mut out = io::stdout();
    if !stdout.is_empty() {
        writeln!(out, "[repl] stdout:")?;
        out.write_all(stdout)?;
        if !stdout.ends_with(b"\n") {
            writeln!(out)?;
        }
        out.flush()?;
    }
    if !stderr.is_empty() {
        let mut err = io::stderr();
        writeln!(err, "[repl] stderr:")?;
        err.write_all(stderr)?;
        if !stderr.ends_with(b"\n") {
            writeln!(err)?;
        }
        err.flush()?;
    }
    Ok(())
}

/// One-line machine state used in success and failure reports
/// 成功与失败报告中使用的单行机器状态
fn machine_state(machine: &Machine) -> String {
    let stop = machine.halt_reason().map_or_else(
        || {
            if machine.budget_exhausted() {
                String::from("budget exhausted")
            } else {
                String::from("running")
            }
        },
        |reason| format!("halt: {reason}"),
    );
    format!(
        "last PC={:#010X}, steps={}/{}, simulated={} us, {stop}",
        machine.pc(),
        machine.steps(),
        machine.budget(),
        machine.time_us()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::infinite_loop_elf;

    #[test]
    fn advance_to_caps_at_the_deadline_and_makes_progress() {
        assert_eq!(advance_to(100, 500, 1_000), 400);
        assert_eq!(advance_to(100, 500, 300), 200);
        assert_eq!(advance_to(100, 50, 1_000), 1);
        assert_eq!(advance_to(100, 500, 100), 0);
    }

    #[test]
    fn enumeration_reports_budget_exhaustion_with_evidence() {
        let elf = infinite_loop_elf();
        let options = MachineOptions {
            arch: Some(ArchArg::Arm),
            max_cycles: 100_000,
            strict_decode: false,
        };
        let mut machine = Machine::new(&elf, &options).expect("machine");
        let Err(error) = open_session(&mut machine, 5_000) else {
            panic!("enumeration must time out");
        };
        let message = error.to_string();
        let EnumerationError::Timeout { steps, pc, .. } = &error else {
            panic!("expected a timeout, got {error:?}");
        };
        assert_eq!(*steps, 5_000);
        assert_eq!(*pc, 0x1000_0000);
        assert!(message.contains("USB enumeration timed out"));
        assert!(message.contains("PC=0x10000000"));
    }

    #[test]
    fn failed_enumeration_keeps_the_host_error() {
        let error = EnumerationError::Failed {
            error: HostError::Timeout { request: 0x80 },
            device: String::from("VID=0x2E8A PID=0x0005"),
            address: 1,
            progress: String::from("stage=enumerating, queued=3"),
        };
        let message = error.to_string();
        assert!(message.contains("Timeout { request: 128 }"));
        assert!(message.contains("VID=0x2E8A"));
        assert!(message.contains("stage=enumerating"));
    }
}
