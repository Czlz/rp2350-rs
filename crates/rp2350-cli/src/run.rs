//! `run` subcommand: build the Stage-4 bus, load firmware, boot and trace
//! `run` 子命令：构建 Stage 4 总线、加载固件、启动并输出轨迹

// 前端地址常量与映像偏移都在 32 位客户机地址空间内换算，这是模拟器前端的固有特性。
// Frontend address constants and image offsets are converted within the 32-bit
// guest address space, inherent to emulator-frontend address arithmetic.
#![allow(clippy::cast_possible_truncation)]

use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::ValueEnum;
use rp2350_arm::CortexM33Core;
use rp2350_bootrom::Rp2350Bootrom;
use rp2350_core::types::CpuId;
use rp2350_core::{
    AddressBus, FLASH_XIP_BASE, FLASH_XIP_NOCACHE_BASE, FirmwareImage, FlashRegion, RamRegion,
    SimpleBus, Uf2Arch, Uf2Image,
};
use rp2350_cpu::{CpuCore, CpuState};
use rp2350_periph::BootPeripherals;
use rp2350_riscv::Hazard3Core;

/// SRAM base address / SRAM 基地址
const SRAM_BASE: u32 = 0x2000_0000;

/// SRAM size (520 KB) / SRAM 大小（520 KB）
const SRAM_SIZE: usize = 520 * 1024;

/// Four-byte UF2 magic prefix / UF2 魔术前缀（四字节）
const UF2_MAGIC: [u8; 4] = [0x55, 0x46, 0x32, 0x0A];

/// Instructions between interrupt polls / 中断轮询间隔（指令数）
const PUMP_INTERVAL: u64 = 64;

/// Maximum number of logged IRQ deliveries / 记录的中断投递条数上限
const IRQ_LOG_LIMIT: usize = 8;

/// Code window treated as one tight loop / 视为同一紧凑循环的代码窗口大小
const SPIN_WINDOW_BYTES: u32 = 32;

/// Consecutive steps inside one window before a spin warning is printed
/// 打印自旋告警前，PC 连续停留在同一代码窗口内的步数
const SPIN_REPORT_STEPS: u64 = 1_000_000;

/// Maximum number of spin warnings per run / 每次运行最多打印的自旋告警条数
const SPIN_REPORT_LIMIT: usize = 4;

/// CPU architecture selection for `run` / `run` 的 CPU 架构选择
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchArg {
    /// Arm Cortex-M33 / Arm Cortex-M33
    Arm,
    /// Hazard3 RISC-V / Hazard3 RISC-V
    RiscV,
}

/// Options for a `run` invocation / `run` 调用选项
#[derive(Debug)]
pub struct RunOptions {
    /// Firmware file (UF2 or ELF) / 固件文件（UF2 或 ELF）
    pub file: PathBuf,
    /// Architecture override / 架构覆盖
    pub arch: Option<ArchArg>,
    /// Instruction budget / 指令预算
    pub max_cycles: u64,
    /// Number of steps printed as a boot trace / 作为启动轨迹打印的步数
    pub trace_steps: usize,
}

/// Build the bus, load firmware and execute until halt or budget exhaustion
/// 构建总线、加载固件并执行到停机或预算耗尽
///
/// # Errors
/// Returns an error when the firmware cannot be read, parsed or loaded, when a
/// bus region cannot be registered, or when the core fails to boot.
/// 固件无法读取、解析或加载，总线区域注册失败，或核心启动失败时返回错误。
pub fn run(options: &RunOptions) -> Result<()> {
    let bytes = std::fs::read(&options.file).with_context(|| {
        format!(
            "cannot read firmware / 无法读取固件: {}",
            options.file.display()
        )
    })?;

    let mut bus = SimpleBus::new();
    // 0x0000_0000..0x0000_7FFF 是独立的 boot ROM 地址区，与 Flash XIP 无关。
    // 这里挂载合成镜像与宿主陷阱桩，启动向量改为从 Flash XIP 基地址读取。
    // 0x0000_0000..0x0000_7FFF is the boot ROM window, separate from flash XIP.
    // Mount the synthetic image with host trap stubs here; boot vectors are now
    // read from the flash XIP base instead.
    let flash = FlashRegion::rp2350_xip();
    let bootrom = Rp2350Bootrom::new(flash.handle());
    bootrom.mount(&mut bus)?;
    // XIP 在缓存窗口 0x1000_0000 与非缓存别名 0x1400_0000 暴露同一份 Flash，
    // 两个区域共享底层存储。
    // XIP exposes the same flash through the cached window at 0x1000_0000 and
    // the non-cached alias at 0x1400_0000; both regions share one backing store.
    let flash_nocache = flash.alias("flash-xip-nocache", FLASH_XIP_NOCACHE_BASE);
    bus.register_region(Box::new(flash))?;
    bus.register_region(Box::new(flash_nocache))?;
    bus.register_region(Box::new(RamRegion::new("sram", SRAM_BASE, SRAM_SIZE)))?;
    let peripherals = BootPeripherals::mount(&mut bus)?;
    // USBCTRL 挂载后，DPSRAM（0x5010_0000）与寄存器窗口（0x5011_0000，含
    // XOR/SET/CLR 别名）由控制器建模，固件对 SIE_CTRL（0x5011_004C）的
    // 读-改-写不再落到 NoDevice。控制器句柄保留到本次运行结束，后续由
    // perfbench 主机桥驱动。
    // After mounting USBCTRL, the DPSRAM window at 0x5010_0000 and the register
    // window at 0x5011_0000 (with XOR/SET/CLR aliases) are modelled, so the
    // firmware read-modify-write of SIE_CTRL (0x5011_004C) no longer hits
    // NoDevice. The controller stays alive for the whole run and is driven by
    // the perfbench host bridge in a later stage.
    let _usb = rp2350_usb::mount(&mut bus).context("USB mount failed / USB 挂载失败")?;

    let (arch, elf_entry) = load_image(&bytes, &mut bus, options.arch)?;

    let (mut core, boot_vectors): (Box<dyn CpuCore>, Option<(u32, u32)>) = match arch {
        ArchArg::Arm => {
            let mut arm = CortexM33Core::new(CpuId::Core0);
            let vectors = if let Some(entry) = elf_entry {
                // ELF 映像没有 Flash 向量表，使用标准 SRAM 栈顶与 ELF 入口。
                // ELF images have no flash vector table; use the SRAM top and ELF entry.
                let sp = SRAM_BASE + SRAM_SIZE as u32;
                arm.boot_at(sp, entry, FLASH_XIP_BASE);
                Some((sp, entry & !1))
            } else {
                // UF2 的复位向量表位于 Flash XIP 基地址，VTOR 也指向那里。
                // UF2 reset vectors live at the flash XIP base, and VTOR follows.
                arm.boot_from(&mut bus, FLASH_XIP_BASE)
                    .context("core boot failed / 核心启动失败")?;
                Some((arm.read_register(13), arm.pc()))
            };
            (Box::new(arm), vectors)
        }
        ArchArg::RiscV => {
            let mut riscv = Hazard3Core::new(CpuId::Core0);
            riscv
                .init(&mut bus)
                .context("core boot failed / 核心启动失败")?;
            (Box::new(riscv), None)
        }
    };

    let mut stdout = std::io::stdout();
    writeln!(stdout, "[run] core: {}", core.description())?;
    if let Some((sp, pc)) = boot_vectors {
        writeln!(stdout, "[run] boot vectors: SP={sp:#010X} PC={pc:#010X}")?;
    }
    writeln!(
        stdout,
        "[run] started: PC={:#010X} SP={:#010X}",
        core.pc(),
        core.read_register(13)
    )?;

    let bootrom = (arch == ArchArg::Arm).then_some(&bootrom);
    let outcome = execute(
        &mut *core,
        &mut bus,
        &peripherals,
        options,
        &mut stdout,
        bootrom,
    )?;
    writeln!(stdout, "[run] halt: {}", outcome.reason)?;
    writeln!(
        stdout,
        "[run] executed {steps} steps, {us} us simulated, final PC={pc:#010X}",
        steps = outcome.steps,
        us = outcome.micros,
        pc = outcome.pc
    )?;
    if !outcome.irqs.is_empty() {
        writeln!(stdout, "[run] irq deliveries (first {IRQ_LOG_LIMIT}):")?;
        for line in &outcome.irqs {
            writeln!(stdout, "{line}")?;
        }
    }
    writeln!(stdout, "[run] uart0: {} bytes", outcome.uart.len())?;
    if !outcome.uart.is_empty() {
        writeln!(stdout, "[run] uart0 output:")?;
        stdout.write_all(&outcome.uart)?;
        if !outcome.uart.ends_with(b"\n") {
            writeln!(stdout)?;
        }
    }
    stdout.flush()?;
    Ok(())
}

/// Result of one execution run / 一次执行的结果
struct Outcome {
    /// Human-readable halt reason / 可读的停机原因
    reason: String,
    /// Steps executed / 已执行步数
    steps: u64,
    /// Simulated microseconds / 仿真的微秒数
    micros: u64,
    /// Final program counter / 最终程序计数器
    pc: u32,
    /// Logged interrupt deliveries / 已记录的中断投递
    irqs: Vec<String>,
    /// Buffered UART output / 已缓冲的 UART 输出
    uart: Vec<u8>,
}

/// Detect the format, load it and return its architecture
/// 检测格式、加载映像并返回其架构
fn load_image(
    bytes: &[u8],
    bus: &mut SimpleBus,
    override_arch: Option<ArchArg>,
) -> Result<(ArchArg, Option<u32>)> {
    if bytes.starts_with(&UF2_MAGIC) {
        let image = Uf2Image::from_bytes(bytes).context("UF2 parse failed / UF2 解析失败")?;
        let family = image.family();
        let detected = family.arch().map_or(ArchArg::Arm, |arch| match arch {
            Uf2Arch::Arm => ArchArg::Arm,
            Uf2Arch::RiscV => ArchArg::RiscV,
        });
        let arch = override_arch.unwrap_or(detected);
        let report = image
            .load_into(bus)
            .context("UF2 load failed / UF2 加载失败")?;
        println!(
            "[run] uf2: family={family} blocks={}/{} flash={} sram={} range={:#010X}..{:#010X}",
            report.blocks_written,
            report.blocks_total,
            report.wrote_flash,
            report.wrote_sram,
            report.min_address,
            report.max_address
        );
        Ok((arch, None))
    } else {
        let image =
            FirmwareImage::from_elf_bytes(bytes).context("ELF parse failed / ELF 解析失败")?;
        image
            .load_into(bus)
            .context("ELF load failed / ELF 加载失败")?;
        let arch = override_arch.unwrap_or(ArchArg::Arm);
        println!(
            "[run] elf: entry={:#010X} segments={}",
            image.entry_point,
            image.segments.len()
        );
        Ok((arch, Some(image.entry_point)))
    }
}

/// Replay PPB system writes into the core and deliver asserted interrupt lines
/// 将 PPB 系统写入回放到核，并投递已断言的中断线
fn pump(core: &mut dyn CpuCore, peripherals: &BootPeripherals, irq_log: &mut Vec<String>) {
    for (addr, value) in peripherals.take_system_writes() {
        core.system_write(addr, value);
    }
    let mask = peripherals.irq_mask();
    for irq in 0..64u16 {
        if mask & (1u64 << irq) == 0 {
            continue;
        }
        core.signal_interrupt(irq);
        if irq_log.len() < IRQ_LOG_LIMIT {
            irq_log.push(format!("[irq] signal {irq} at PC={:#010X}", core.pc()));
        }
    }
}

/// Step the core, printing a short trace and draining UART output
/// 单步执行核心，打印简短轨迹并抽取 UART 输出
fn execute(
    core: &mut dyn CpuCore,
    bus: &mut SimpleBus,
    peripherals: &BootPeripherals,
    options: &RunOptions,
    stdout: &mut dyn Write,
    bootrom: Option<&Rp2350Bootrom>,
) -> Result<Outcome> {
    let mut steps = 0u64;
    let mut reason = String::from("instruction budget exhausted / 指令预算耗尽");
    let mut uart = Vec::new();
    let mut irqs = Vec::new();
    let mut spin_base = core.pc();
    let mut spin_steps = 0u64;
    let mut spin_reports = 0usize;

    for index in 0..options.trace_steps {
        let pc = core.pc();
        if let Some(bootrom) = bootrom {
            if bootrom.handles(pc) {
                writeln!(stdout, "[trace] {index:03} PC={pc:#010X} bootrom-trap")?;
                if let Some(outcome) = bootrom.dispatch(core, bus)? {
                    steps += 1;
                    if let Some(halt) = outcome.halt {
                        reason = format!("bootrom halt: {halt}");
                        return Ok(finish(peripherals, core, steps, reason, irqs, uart));
                    }
                    pump(core, peripherals, &mut irqs);
                    continue;
                }
            }
        }
        let mut cycles = 0u64;
        let opcode = bus.read_u16(pc, &mut cycles).unwrap_or(0xFFFF);
        writeln!(stdout, "[trace] {index:03} PC={pc:#010X} op={opcode:#06X}")?;
        match core.step(bus) {
            Ok(CpuState::Running) => steps += 1,
            Ok(state) => {
                reason = format!("core entered state {state:?}");
                return Ok(finish(peripherals, core, steps, reason, irqs, uart));
            }
            Err(error) => {
                reason = error.to_string();
                return Ok(finish(peripherals, core, steps, reason, irqs, uart));
            }
        }
        pump(core, peripherals, &mut irqs);
    }

    let mut advanced = 0u64;
    let mut pumped = 0u64;
    while steps < options.max_cycles {
        let mut handled = false;
        if let Some(bootrom) = bootrom {
            if bootrom.handles(core.pc()) {
                if let Some(outcome) = bootrom.dispatch(core, bus)? {
                    steps += 1;
                    handled = true;
                    if let Some(halt) = outcome.halt {
                        reason = format!("bootrom halt: {halt}");
                        break;
                    }
                }
            }
        }
        if !handled {
            match core.step(bus) {
                Ok(CpuState::Running) => steps += 1,
                Ok(state) => {
                    reason = format!("core entered state {state:?}");
                    break;
                }
                Err(error) => {
                    reason = error.to_string();
                    break;
                }
            }
        }
        // 紧凑自旋通常意味着固件在轮询一个永远不变的 MMIO 值（未建模设备静默
        // 读 0）或执行延时循环，会无声耗尽指令预算。PC 连续停在同一小窗口内
        // 达到阈值就告警一次，并给出窗口范围以便反汇编定位。
        // A tight spin usually means the firmware polls a register that never
        // changes (unmodelled devices read zero) or runs a delay loop, silently
        // burning the budget. Warn once when the PC stays inside one small window
        // and print the window so the spin point can be disassembled.
        let pc = core.pc();
        if pc.wrapping_sub(spin_base) < SPIN_WINDOW_BYTES {
            spin_steps += 1;
            if spin_steps == SPIN_REPORT_STEPS && spin_reports < SPIN_REPORT_LIMIT {
                spin_reports += 1;
                writeln!(
                    stdout,
                    "[run] tight spin: PC stayed in {:#010X}..{:#010X} for {spin_steps} steps, \
                     likely polling a value that never changes / 疑似轮询等待",
                    spin_base,
                    spin_base + SPIN_WINDOW_BYTES
                )?;
            }
        } else {
            spin_base = pc;
            spin_steps = 0;
        }
        if steps - pumped >= PUMP_INTERVAL {
            pump(core, peripherals, &mut irqs);
            pumped = steps;
        }
        if steps - advanced >= 10_000 {
            peripherals.advance(steps - advanced);
            advanced = steps;
            uart.extend(peripherals.take_uart_output());
        }
    }
    peripherals.advance(steps - advanced);
    uart.extend(peripherals.take_uart_output());
    Ok(finish(peripherals, core, steps, reason, irqs, uart))
}

/// Collect final state and UART output / 收集最终状态与 UART 输出
fn finish(
    peripherals: &BootPeripherals,
    core: &dyn CpuCore,
    steps: u64,
    reason: String,
    irqs: Vec<String>,
    mut uart: Vec<u8>,
) -> Outcome {
    uart.extend(peripherals.take_uart_output());
    Outcome {
        reason,
        steps,
        micros: peripherals.time_us(),
        pc: core.pc(),
        irqs,
        uart,
    }
}
