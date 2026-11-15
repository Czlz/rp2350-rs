//! `repl` subcommand: drive the USB CDC raw REPL of a booted machine
//! `repl` 子命令：驱动已启动机器的 USB CDC raw REPL
//!
//! Stock `MicroPython` RP2350 firmware exposes its REPL only over USB CDC, so the
//! host must first let the guest boot, enumerate the emulated CDC device and
//! then speak the raw REPL protocol over the bulk endpoints discovered by
//! [`UsbHostBridge`]. / 原生 `MicroPython` RP2350 固件只在 USB CDC 上暴露 REPL，
//! 因此宿主必须先让客户机启动、枚举模拟 CDC 设备，再通过
//! [`UsbHostBridge`] 发现的批量端点使用 raw REPL 协议。

use std::collections::VecDeque;
use std::fmt;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, TryRecvError};
use std::thread;

use anyhow::{Context, Result, anyhow, bail};
use rp2350_usb_host::repl::BANNER_LINE;
use rp2350_usb_host::{
    ByteLink, CdcChannel, CdcEndpoints, CdcLink, HostError, HostProgress, RawReplSession,
    UsbHostBridge,
};

use crate::run::{
    ArchArg, Machine, MachineOptions, PsramOptions, SYSTEM_RESET_MESSAGE, fault_tail_lines,
};

/// Default total instruction budget for `repl` / `repl` 的默认总指令预算
pub const DEFAULT_MAX_CYCLES: u64 = 300_000_000;

/// Default budget allowed for USB enumeration / USB 枚举的默认预算
pub const DEFAULT_ENUM_CYCLES: u64 = 100_000_000;

/// Default simulation step for the stdio bridge (µs) / stdio 桥接的默认模拟步长（微秒）
///
/// 500 µs is converted to guest cycles with the live `CLK_SYS` supplied to
/// [`CdcLink`] (75 000 cycles at 150 MHz, 126 000 at 252 MHz): large enough for
/// throughput, small enough for interactive latency.
/// 500 微秒按传给 [`CdcLink`] 的实时 `CLK_SYS` 换算为客户机周期（150 MHz 下
/// 75 000、252 MHz 下 126 000）：兼顾吞吐与交互延迟。
pub const STDIO_CHUNK_MICROS: u64 = 500;

/// Bytes read per stdin chunk before handing them to the bridge
/// 每次从 stdin 读取并交给桥接的字节数
const STDIO_READ_CHUNK: usize = 4 * 1024;

/// Bounded queue depth: 16 × 4 KiB = 64 KiB of in-flight stdin data
/// 有界队列深度：16 × 4 KiB = 64 KiB 在途 stdin 数据
const STDIO_QUEUE_SLOTS: usize = 16;

/// Simulated drain time after stdin EOF so late guest output is not truncated
/// stdin EOF 后的模拟排空时间，避免截断客户机的迟到输出
const STDIO_DRAIN_MICROS: u64 = 50_000;

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
    /// Transparent stdin/stdout bridge instead of the CLI-driven raw REPL
    /// 使用 stdin/stdout 透明桥接，而不是由 CLI 驱动 raw REPL
    pub stdio: bool,
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
    /// PSRAM window to mount behind XIP chip select 1 / 在 XIP 片选 1 后挂载的 PSRAM 窗口
    pub psram: Option<PsramOptions>,
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
    /// The guest requested `AIRCR.SYSRESETREQ` while enumerating
    /// 客户机在枚举期间请求 `AIRCR.SYSRESETREQ`
    SystemReset {
        /// Executed steps / 已执行步数
        steps: u64,
        /// Last program counter / 最后程序计数器
        pc: u32,
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
            Self::SystemReset { steps, pc } => write!(
                f,
                "{SYSTEM_RESET_MESSAGE} during enumeration / 枚举期间发生系统复位请求 \
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
/// Returns [`EnumerationError`] when the guest halts, requests a system reset,
/// detaches, fails a control transfer, or exhausts the enumeration/machine
/// budget. / 客户机停机、请求系统复位、设备消失、控制传输失败或枚举/机器预算
/// 耗尽时返回 [`EnumerationError`]。
pub fn open_session<'a>(
    machine: &'a mut Machine,
    enum_cycles: u64,
) -> Result<OpenSession<'a>, EnumerationError> {
    let start = machine.now_cycles();
    let deadline = start.saturating_add(enum_cycles);
    // 枚举与后续 CDC 传输都使用客户机当前的时钟树：CLK_SYS 换算周期，
    // CLK_USB 驱动 1 ms SOF。
    // Enumeration and the following CDC data phase use the guest's clock tree:
    // CLK_SYS converts cycles and CLK_USB drives the 1 ms SOF.
    let sysclk = machine.peripherals().sysclk_hz();
    let usbclk = machine.peripherals().usbclk_hz();
    let mut bridge = UsbHostBridge::with_clock(machine.usb().clone(), sysclk, usbclk);
    let mut last = String::from("not polled");
    let endpoints = loop {
        if let Some(reason) = machine.halt_reason() {
            return Err(EnumerationError::Halted {
                steps: machine.steps(),
                pc: machine.pc(),
                reason: reason.to_string(),
            });
        }
        // 复位请求后机器不再推进，继续等待枚举只会空转；`repl` 的策略是报告并
        // 停止，而不是像 `run` 那样自动重启。
        // The machine stops advancing after a reset request, so waiting for
        // enumeration would only spin; `repl` reports the request and stops
        // instead of rebooting like `run`.
        if machine.system_reset_pending() {
            return Err(EnumerationError::SystemReset {
                steps: machine.steps(),
                pc: machine.pc(),
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

    let channel = CdcChannel::with_clock(machine.usb().clone(), endpoints, sysclk, usbclk);
    let runner: Box<dyn FnMut(u64) + 'a> = Box::new(move |cycles| machine.run_cycles(cycles));
    let link = CdcLink::with_clock(channel, runner, sysclk);
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

/// Transparently bridge `input`/`output` to a CDC byte link
/// 将 `input`/`output` 透明桥接到 CDC 字节链路
///
/// A background thread performs blocking reads on `input` (a Windows pipe or a
/// terminal) and forwards 4 KiB chunks through a bounded channel (16 slots,
/// 64 KiB in flight); the main loop drains that channel into [`ByteLink::send`],
/// copies [`ByteLink::receive`] data to `output` and flushes it, then calls
/// [`ByteLink::advance`] to run the guest. After stdin EOF the loop keeps
/// simulating for a short bounded drain window so a late banner or prompt is
/// not truncated, then returns. The CLI never speaks the raw REPL protocol in
/// this mode, so guest bytes are the only bytes on `output`.
/// 后台线程对 `input`（Windows 管道或终端）执行阻塞读取，并以 4 KiB 块经有界
/// 通道（16 槽，最多 64 KiB 在途）转交主循环；主循环把通道数据排入
/// [`ByteLink::send`]，把 [`ByteLink::receive`] 的数据写入 `output` 并 flush，
/// 再调用 [`ByteLink::advance`] 推进客户机。stdin EOF 后会继续模拟一个很短的
/// 有界排空窗口，以免截断迟到的 banner 或提示符，然后返回。该模式下 CLI 完全
/// 不参与 raw REPL 协议，因此 `output` 上只有客户机字节。
///
/// `chunk_micros` is the per-iteration simulated step (clamped to at least 1):
/// 100–1000 µs balances throughput and interactive latency; smaller values
/// improve latency but increase host loop overhead.
/// `chunk_micros` 是每轮的模拟步长（至少为 1）：100–1000 微秒兼顾吞吐与交互
/// 延迟；更小的值降低延迟但增加宿主循环开销。
///
/// Returns the number of stdin bytes handed to the link.
/// 返回已交给链路的 stdin 字节数。
///
/// # Errors
/// Returns the stdin reader's error, or any error from writing/flushing
/// `output` (a closed downstream normally surfaces as
/// [`io::ErrorKind::BrokenPipe`]). / 返回 stdin 读取错误，或写/flush `output`
/// 的错误（下游关闭通常表现为 [`io::ErrorKind::BrokenPipe`]）。
// 按任务约定保留按值传入的 `output`；它只被可变借用以写字节。
// Keep `output` by value per the task signature; it is only used mutably.
#[allow(clippy::needless_pass_by_value)]
pub fn bridge_stdio<L, R, W>(
    link: &mut L,
    input: R,
    mut output: W,
    chunk_micros: u64,
) -> io::Result<u64>
where
    L: ByteLink,
    R: Read + Send + 'static,
    W: Write,
{
    let step = chunk_micros.max(1);
    let (sender, receiver) = mpsc::sync_channel::<io::Result<Vec<u8>>>(STDIO_QUEUE_SLOTS);
    let reader = thread::Builder::new()
        .name(String::from("rp2350-stdio-reader"))
        .spawn(move || read_stdin(input, &sender))?;
    let result = pump_stdio(link, &receiver, &mut output, step);
    drop(receiver);
    // 仅当读线程已因 EOF/错误退出时 join，避免 stdout 关闭后卡在阻塞读。
    // Join only after the reader has exited; otherwise a closed stdout could
    // leave us waiting on a still-open stdin pipe.
    if result.is_ok() {
        let _ = reader.join();
    }
    result
}

/// Forward blocking stdin reads into the bounded channel
/// 将阻塞的 stdin 读取转发到有界通道
fn read_stdin<R: Read>(mut input: R, sender: &mpsc::SyncSender<io::Result<Vec<u8>>>) {
    let mut buffer = vec![0u8; STDIO_READ_CHUNK];
    loop {
        match input.read(&mut buffer) {
            Ok(0) => return,
            Ok(read) => {
                if sender.send(Ok(buffer[..read].to_vec())).is_err() {
                    return;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => {
                let _ = sender.send(Err(error));
                return;
            }
        }
    }
}

/// Main stdio bridge loop / stdio 桥接主循环
fn pump_stdio<L, W>(
    link: &mut L,
    receiver: &mpsc::Receiver<io::Result<Vec<u8>>>,
    output: &mut W,
    step_micros: u64,
) -> io::Result<u64>
where
    L: ByteLink,
    W: Write,
{
    let mut backlog = VecDeque::new();
    let mut scratch = [0u8; STDIO_READ_CHUNK];
    let mut total = 0u64;
    let mut eof = false;
    let mut drained = 0u64;
    loop {
        while !eof {
            match receiver.try_recv() {
                Ok(Ok(chunk)) => {
                    total += chunk.len() as u64;
                    backlog.extend(chunk);
                }
                Ok(Err(error)) => return Err(error),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => eof = true,
            }
        }
        while !backlog.is_empty() {
            let accepted = link.send(backlog.make_contiguous());
            let accepted = accepted.min(backlog.len());
            backlog.drain(..accepted);
            if accepted == 0 {
                break;
            }
        }
        let mut written = 0usize;
        loop {
            // 防御性截断：链路违反契约返回超长计数时也不越界。
            // Defensive clamp: never index past the scratch buffer if a link
            // reports more bytes than it was given room for.
            let read = link.receive(&mut scratch).min(scratch.len());
            if read == 0 {
                break;
            }
            output.write_all(&scratch[..read])?;
            written += read;
        }
        if written > 0 {
            output.flush()?;
        }
        if eof && backlog.is_empty() {
            if drained >= STDIO_DRAIN_MICROS {
                return Ok(total);
            }
            drained = drained.saturating_add(step_micros);
        }
        link.advance(step_micros);
    }
}

/// Destination for `[repl]` diagnostics / `[repl]` 诊断信息的输出目标
#[derive(Clone, Copy)]
enum Report {
    /// Normal stdout / 常规标准输出
    Stdout,
    /// Stderr, keeping stdout guest-only / 标准错误，保持 stdout 只有客户机字节
    Stderr,
}

impl Report {
    /// Emit one prefixed diagnostic line / 输出一行带前缀的诊断信息
    fn line(self, text: &str) {
        match self {
            Self::Stdout => println!("[repl] {text}"),
            Self::Stderr => eprintln!("[repl] {text}"),
        }
    }
}

/// `0` selects an unlimited budget; any other value is kept as-is
/// `0` 表示不限制预算；其他值保持不变
fn effective_max_cycles(max_cycles: u64) -> u64 {
    if max_cycles == 0 {
        u64::MAX
    } else {
        max_cycles
    }
}

/// Open the raw REPL or bridge stdio, optionally execute source, then finish
/// 打开 raw REPL 或桥接 stdio，可选执行源码，然后收尾
///
/// In `--stdio` mode the CLI never drives the raw REPL protocol: stdin bytes go
/// straight to the CDC OUT endpoint and CDC IN bytes go straight to stdout.
/// All `[repl]` diagnostics move to stderr so stdout stays guest-only.
/// `--stdio` 模式下 CLI 不参与 raw REPL 协议：stdin 字节直接进入 CDC OUT 端点，
/// CDC IN 字节直接写到 stdout；所有 `[repl]` 诊断改走 stderr，保持 stdout 只有
/// 客户机字节。
///
/// `--max-cycles 0` means "no limit" (`u64::MAX`); the flag's default remains
/// [`DEFAULT_MAX_CYCLES`], so `repl --exec` keeps its historical budget.
/// `--max-cycles 0` 表示不限制（`u64::MAX`）；该选项默认值仍为
/// [`DEFAULT_MAX_CYCLES`]，因此 `repl --exec` 保持原有预算行为。
///
/// A guest `AIRCR.SYSRESETREQ` is reported as
/// [`SYSTEM_RESET_MESSAGE`] and stops the subcommand with an error; unlike
/// `run`, `repl` never reboots the machine behind the user's back.
/// 客户机的 `AIRCR.SYSRESETREQ` 以 [`SYSTEM_RESET_MESSAGE`] 报告，并让子命令带错误
/// 停止；与 `run` 不同，`repl` 绝不在用户不知情时重启机器。
///
/// # Errors
/// Returns an error when the firmware cannot be loaded, USB enumeration fails,
/// the raw REPL protocol times out, executed source writes to stderr, or
/// `--stdio` is combined with `--exec`. / 固件无法加载、USB 枚举失败、raw REPL
/// 协议超时、执行源码写入标准错误，或 `--stdio` 与 `--exec` 同时给出时返回错误。
pub fn run(options: &ReplOptions) -> Result<()> {
    if options.stdio && options.exec.is_some() {
        bail!("--stdio conflicts with --exec / --stdio 与 --exec 冲突");
    }
    let bytes = std::fs::read(&options.file).with_context(|| {
        format!(
            "cannot read firmware / 无法读取固件: {}",
            options.file.display()
        )
    })?;
    let machine_options = MachineOptions {
        arch: options.arch,
        max_cycles: effective_max_cycles(options.max_cycles),
        strict_decode: options.strict_decode,
        psram: options.psram,
    };
    let mut machine = Machine::new(&bytes, &machine_options)?;
    // TinyUSB 的 EP0 没有端点控制寄存器，`hw_endpoint_start_next_buffer()` 每次
    // 控制传输都会写 `*ep->endpoint_control`（EP0 为 NULL）。真实 RP2350 的掩膜
    // ROM 会忽略地址 0 的写入；合成 boot ROM 区域默认拒绝写入，会让客户机在首个
    // SETUP 阶段以 `Bus error at 0x00000000` 停机。这里换成写入丢弃的掩膜 ROM
    // 视图，再开始枚举。
    // TinyUSB's EP0 has no endpoint-control register, so
    // `hw_endpoint_start_next_buffer()` writes `*ep->endpoint_control` (NULL for
    // EP0) on every control transfer. Real RP2350 mask ROM ignores stores to
    // address 0; the synthetic boot ROM region rejects them, which halted the
    // guest at the first SETUP with `Bus error at 0x00000000`. Swap in the
    // write-dropping mask-ROM view before enumeration starts.
    rp2350_usb_host::mask_rom::install(&mut machine.bus)
        .context("mask ROM view install failed / 掩膜 ROM 视图安装失败")?;
    let stdio = options.stdio;
    let report = if stdio {
        Report::Stderr
    } else {
        Report::Stdout
    };
    report.line(machine.image_description());
    report.line(&format!("core: {}", machine.core_description()));

    let source = options.exec.clone();
    let result = (|| -> Result<()> {
        let mut open =
            open_session(&mut machine, options.enum_cycles).map_err(|error| anyhow!(error))?;
        report.line(&format!(
            "USB CDC configured: IN=0x{:02X}/{} OUT=0x{:02X}/{} interface={}",
            open.endpoints.in_address,
            open.endpoints.in_max_packet,
            open.endpoints.out_address,
            open.endpoints.out_max_packet,
            open.endpoints.interface
        ));
        if stdio {
            return run_stdio_bridge(open.session.link_mut());
        }
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
    // 复位请求会让机器停止推进：`repl` 的策略是报告并停止，不自动重启（自动重启
    // 只属于 `run`）。此时即使 raw REPL 因客户机停走而超时，也以复位请求为准。
    // A system reset request stops the machine: `repl` reports it and stops
    // instead of rebooting (automatic restarts belong to `run` only). Even if
    // the raw REPL timed out because the guest stopped, the reset is the reason.
    let system_reset = machine.take_system_reset_request();
    machine.finalize();
    if system_reset {
        report.line(&format!(
            "{SYSTEM_RESET_MESSAGE}; stopping / 检测到系统复位请求，停止"
        ));
        diagnose_failure(&mut machine);
        let state = machine_state(&machine);
        return Err(anyhow::Error::msg(SYSTEM_RESET_MESSAGE).context(state));
    }

    let state = machine_state(&machine);
    match result {
        Ok(()) => {
            report.line(&state);
            Ok(())
        }
        Err(error) => {
            diagnose_failure(&mut machine);
            Err(error.context(state))
        }
    }
}

/// Drive the transparent stdio bridge and treat a closed stdout as success
/// 驱动透明 stdio 桥接，并把 stdout 关闭视为正常结束
///
/// # Errors
/// Propagates non-[`io::ErrorKind::BrokenPipe`] bridge failures.
/// 传播除 [`io::ErrorKind::BrokenPipe`] 之外的桥接失败。
fn run_stdio_bridge<L: ByteLink>(link: &mut L) -> Result<()> {
    match bridge_stdio(link, io::stdin(), io::stdout().lock(), STDIO_CHUNK_MICROS) {
        Ok(bytes) => {
            eprintln!("[repl] stdio bridge ended after {bytes} stdin bytes / stdio 桥接结束");
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => {
            eprintln!(
                "[repl] stdout closed, stopping stdio bridge / \
                 标准输出已关闭，停止 stdio 桥接"
            );
            Ok(())
        }
        Err(error) => Err(error).context("stdio bridge failed / stdio 桥接失败"),
    }
}

/// Dump guest fault diagnostics to stderr / 将客户机故障诊断输出到 stderr
fn diagnose_failure(machine: &mut Machine) {
    // 客户机故障往往只留下一个最终 PC；附上停机前的指令轨迹，
    // 才能判断是哪条分支或异常返回跑飞。
    // A guest fault often leaves only a final PC; attach the recent
    // instruction history so the offending branch is identifiable.
    for line in fault_tail_lines(machine) {
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
            psram: None,
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

    #[test]
    fn zero_budget_means_unlimited() {
        assert_eq!(effective_max_cycles(0), u64::MAX);
        assert_eq!(effective_max_cycles(DEFAULT_MAX_CYCLES), DEFAULT_MAX_CYCLES);
    }

    /// Scriptable in-memory link used by the stdio bridge tests
    /// stdio 桥接测试使用的可编排内存链路
    struct FakeLink {
        sent: Vec<u8>,
        incoming: VecDeque<u8>,
        advances: u64,
        now: u64,
        accept: Option<usize>,
        banner_at: Option<u64>,
        echo: bool,
        echoed: usize,
    }

    impl FakeLink {
        fn new() -> Self {
            Self {
                sent: Vec::new(),
                incoming: VecDeque::new(),
                advances: 0,
                now: 0,
                accept: None,
                banner_at: None,
                echo: false,
                echoed: 0,
            }
        }

        fn banner_after(advances: u64) -> Self {
            let mut link = Self::new();
            link.banner_at = Some(advances);
            link
        }
    }

    impl ByteLink for FakeLink {
        fn send(&mut self, bytes: &[u8]) -> usize {
            let accepted = self
                .accept
                .map_or(bytes.len(), |limit| limit.min(bytes.len()));
            self.sent.extend_from_slice(&bytes[..accepted]);
            accepted
        }

        fn receive(&mut self, out: &mut [u8]) -> usize {
            let mut copied = 0;
            for slot in out.iter_mut() {
                let Some(byte) = self.incoming.pop_front() else {
                    break;
                };
                *slot = byte;
                copied += 1;
            }
            copied
        }

        fn advance(&mut self, micros: u64) {
            self.advances += 1;
            self.now = self.now.saturating_add(micros);
            if self.echo {
                self.incoming
                    .extend(self.sent[self.echoed..].iter().copied());
                self.echoed = self.sent.len();
            }
            if self.banner_at.is_some_and(|at| self.advances >= at) {
                self.incoming.extend(b"raw REPL; CTRL-B to exit\r\n>");
                self.banner_at = None;
            }
        }

        fn now_micros(&self) -> u64 {
            self.now
        }
    }

    /// Output sink that records bytes and flush calls / 记录字节与 flush 次数的输出端
    #[derive(Default)]
    struct RecordingWriter {
        data: Vec<u8>,
        flushes: usize,
    }

    impl Write for RecordingWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.data.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flushes += 1;
            Ok(())
        }
    }

    struct BrokenWriter;

    impl Write for BrokenWriter {
        fn write(&mut self, _bytes: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct BrokenReader;

    impl Read for BrokenReader {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("stdin failed"))
        }
    }

    #[test]
    fn bridge_echoes_stdin_and_banner_after_advances() {
        let mut link = FakeLink::banner_after(2);
        link.echo = true;
        let mut output = RecordingWriter::default();
        let read = bridge_stdio(
            &mut link,
            io::Cursor::new(b"\r\x01".to_vec()),
            &mut output,
            100,
        )
        .expect("bridge");
        assert_eq!(read, 2);
        assert_eq!(link.sent, b"\r\x01");
        assert!(link.advances >= 2);
        assert!(output.data.windows(2).any(|window| window == b"\r\x01"));
        let banner = b"raw REPL; CTRL-B to exit\r\n>";
        assert!(
            output
                .data
                .windows(banner.len())
                .any(|window| window == banner)
        );
        assert!(output.flushes > 0, "guest output must be flushed");
    }

    #[test]
    fn bridge_sends_every_cursor_byte_before_eof_returns() {
        let payload = vec![0xA5u8; 1_024];
        let mut link = FakeLink::new();
        link.accept = Some(7);
        let read = bridge_stdio(&mut link, io::Cursor::new(payload.clone()), io::sink(), 50)
            .expect("bridge");
        assert_eq!(read, 1_024);
        assert_eq!(link.sent, payload);
        assert!(link.advances >= 1_024 / 7);
    }

    #[test]
    fn bridge_keeps_large_input_across_the_bounded_queue() {
        let payload: Vec<u8> = (0..256 * 1024)
            .map(|index| u8::try_from(index % 251).expect("byte"))
            .collect();
        let mut link = FakeLink::new();
        let read = bridge_stdio(&mut link, io::Cursor::new(payload.clone()), io::sink(), 10)
            .expect("bridge");
        assert_eq!(read, 256 * 1024);
        assert_eq!(link.sent.len(), payload.len());
        assert_eq!(link.sent, payload);
    }

    #[test]
    fn bridge_reports_broken_pipe_without_panicking() {
        let mut link = FakeLink::banner_after(1);
        let error = bridge_stdio(&mut link, io::Cursor::new(b"x".to_vec()), BrokenWriter, 10)
            .expect_err("stdout failure must surface");
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn bridge_propagates_stdin_read_errors() {
        let mut link = FakeLink::new();
        let error = bridge_stdio(&mut link, BrokenReader, io::sink(), 10)
            .expect_err("stdin failure must surface");
        assert_eq!(error.kind(), io::ErrorKind::Other);
    }
}
