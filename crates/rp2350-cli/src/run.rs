//! `run` subcommand: build the Stage-4 bus, load firmware, boot and trace
//! `run` 子命令：构建 Stage 4 总线、加载固件、启动并输出轨迹

// 前端地址常量与映像偏移都在 32 位客户机地址空间内换算，这是模拟器前端的固有特性。
// Frontend address constants and image offsets are converted within the 32-bit
// guest address space, inherent to emulator-frontend address arithmetic.
#![allow(clippy::cast_possible_truncation)]

use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use clap::ValueEnum;
use rp2350_arm::CortexM33Core;
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

/// Boot ROM stub base / Boot ROM 桩基地址
const BOOTROM_STUB_BASE: u32 = 0x0000_0000;

/// Boot ROM stub size (4 KB) / Boot ROM 桩大小（4 KB）
const BOOTROM_STUB_SIZE: usize = 0x1000;

/// Number of flash bytes mirrored into the boot stub (vectors SP..IRQ51)
/// 镜像到启动桩的 Flash 字节数（SP 至 IRQ51 向量）
const VECTOR_BYTES: usize = 0x140;

/// Four-byte UF2 magic prefix / UF2 魔术前缀（四字节）
const UF2_MAGIC: [u8; 4] = [0x55, 0x46, 0x32, 0x0A];

/// Instructions between interrupt polls / 中断轮询间隔（指令数）
const PUMP_INTERVAL: u64 = 64;

/// Maximum number of logged IRQ deliveries / 记录的中断投递条数上限
const IRQ_LOG_LIMIT: usize = 8;

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
    // 启动桩保存 Flash 向量表的镜像，使现有 `boot` 契约（从 0x0 取向量）
    // 无需改动 Arm crate 即可直接启动 Flash 中的映像。
    // The stub holds a mirror of the flash vector table so the existing `boot`
    // contract (vectors at 0x0) can start a flash image without touching the Arm crate.
    bus.register_region(Box::new(RamRegion::new(
        "bootrom-stub",
        BOOTROM_STUB_BASE,
        BOOTROM_STUB_SIZE,
    )))?;
    // XIP 在缓存窗口 0x1000_0000 与非缓存别名 0x1400_0000 暴露同一份 Flash，
    // 两个区域共享底层存储。
    // XIP exposes the same flash through the cached window at 0x1000_0000 and
    // the non-cached alias at 0x1400_0000; both regions share one backing store.
    let flash = FlashRegion::rp2350_xip();
    let flash_nocache = flash.alias("flash-xip-nocache", FLASH_XIP_NOCACHE_BASE);
    bus.register_region(Box::new(flash))?;
    bus.register_region(Box::new(flash_nocache))?;
    bus.register_region(Box::new(RamRegion::new("sram", SRAM_BASE, SRAM_SIZE)))?;
    let peripherals = BootPeripherals::mount(&mut bus)?;

    let (arch, elf_entry) = load_image(&bytes, &mut bus, options.arch)?;
    let vectors = if arch == ArchArg::Arm {
        Some(prepare_arm_vectors(&mut bus, elf_entry)?)
    } else {
        None
    };

    let mut core: Box<dyn CpuCore> = match arch {
        ArchArg::Arm => Box::new(CortexM33Core::new(CpuId::Core0)),
        ArchArg::RiscV => Box::new(Hazard3Core::new(CpuId::Core0)),
    };
    core.init(&mut bus)
        .context("core boot failed / 核心启动失败")?;

    let mut stdout = std::io::stdout();
    writeln!(stdout, "[run] core: {}", core.description())?;
    if let Some((sp, pc)) = vectors {
        writeln!(stdout, "[run] boot vectors: SP={sp:#010X} PC={pc:#010X}")?;
    }
    writeln!(
        stdout,
        "[run] started: PC={:#010X} SP={:#010X}",
        core.pc(),
        core.read_register(13)
    )?;

    let outcome = execute(&mut *core, &mut bus, &peripherals, options, &mut stdout)?;
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

/// Populate the boot stub and return the applied (SP, PC) pair
/// 填充启动桩并返回应用的 (SP, PC)
fn prepare_arm_vectors(bus: &mut SimpleBus, elf_entry: Option<u32>) -> Result<(u32, u32)> {
    let mut cycles = 0u64;
    if let Some(entry) = elf_entry {
        // ELF 映像没有 Flash 向量表，使用标准 SRAM 栈顶与 ELF 入口。
        // ELF images have no flash vector table; use the SRAM top and ELF entry.
        let sp = SRAM_BASE + SRAM_SIZE as u32;
        bus.write_u32(BOOTROM_STUB_BASE, sp, &mut cycles)?;
        bus.write_u32(BOOTROM_STUB_BASE + 4, entry | 1, &mut cycles)?;
        return Ok((sp, entry & !1));
    }

    let mut vectors = [0u8; VECTOR_BYTES];
    for (index, slot) in vectors.iter_mut().enumerate() {
        *slot = bus.read_u8(FLASH_XIP_BASE + index as u32, &mut cycles)?;
    }
    if vectors[..8].iter().all(|byte| *byte == 0) {
        return Err(anyhow!(
            "flash vector table at {FLASH_XIP_BASE:#010X} is empty / Flash 向量表为空"
        ));
    }
    for (index, byte) in vectors.iter().enumerate() {
        bus.load_u8(BOOTROM_STUB_BASE + index as u32, *byte, &mut cycles)?;
    }
    let sp = u32::from_le_bytes([vectors[0], vectors[1], vectors[2], vectors[3]]);
    let reset = u32::from_le_bytes([vectors[4], vectors[5], vectors[6], vectors[7]]);
    Ok((sp, reset & !1))
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
) -> Result<Outcome> {
    let mut steps = 0u64;
    let mut reason = String::from("instruction budget exhausted / 指令预算耗尽");
    let mut uart = Vec::new();
    let mut irqs = Vec::new();

    for index in 0..options.trace_steps {
        let pc = core.pc();
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
