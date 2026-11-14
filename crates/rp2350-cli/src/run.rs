//! `run` subcommand: build the Stage-4 bus, load firmware, boot and trace
//! `run` 子命令：构建 Stage 4 总线、加载固件、启动并输出轨迹

// 前端地址常量与映像偏移都在 32 位客户机地址空间内换算，这是模拟器前端的固有特性。
// Frontend address constants and image offsets are converted within the 32-bit
// guest address space, inherent to emulator-frontend address arithmetic.
#![allow(clippy::cast_possible_truncation)]

use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::ValueEnum;
use rp2350_arm::CortexM33Core;
use rp2350_bootrom::Rp2350Bootrom;
use rp2350_core::types::CpuId;
use rp2350_core::{
    AddressBus, FLASH_XIP_BASE, FLASH_XIP_NOCACHE_BASE, FirmwareImage, FlashRegion, RamRegion,
    SimpleBus, Uf2Arch, Uf2Image, XipMaintenanceRegion,
};
use rp2350_cpu::{CpuCore, CpuState};
use rp2350_periph::{BootPeripherals, Core1Launch};
use rp2350_riscv::Hazard3Core;
use rp2350_usb::{Controller, USBCTRL_IRQ};

/// SRAM base address / SRAM 基地址
const SRAM_BASE: u32 = 0x2000_0000;

/// SRAM size (520 KB) / SRAM 大小（520 KB）
const SRAM_SIZE: usize = 520 * 1024;

/// Four-byte UF2 magic prefix / UF2 魔术前缀（四字节）
const UF2_MAGIC: [u8; 4] = [0x55, 0x46, 0x32, 0x0A];

/// Default instruction budget for `run` / `run` 的默认指令预算
pub const DEFAULT_RUN_MAX_CYCLES: u64 = 20_000_000;

/// Instructions between interrupt polls / 中断轮询间隔（指令数）
const PUMP_INTERVAL: u64 = 64;

/// Instructions between simulated-time advances / 推进模拟时间的指令间隔
const ADVANCE_INTERVAL: u64 = 10_000;

/// DMA data units moved per pump batch / 每次泵送批次搬运的 DMA 数据单元数
///
/// The DMA controller visits its 16 channels round-robin, so a batch of 16
/// units can move at most one unit per channel and can never run longer than
/// one unit for a single busy channel. This keeps the CPU ahead of the DMA
/// engine without a per-channel scheduler.
/// DMA 控制器按轮转顺序访问 16 个通道，因此一批 16 个单元最多每个通道一个
/// 单元，单一忙碌通道也不会超过一个单元。这样无需逐通道调度即可保证 CPU 始终
/// 领先于 DMA 引擎。
const DMA_PUMP_TRANSFERS: u64 = 64;

/// Maximum number of logged IRQ deliveries / 记录的中断投递条数上限
const IRQ_LOG_LIMIT: usize = 8;

/// Code window treated as one tight loop / 视为同一紧凑循环的代码窗口大小
const SPIN_WINDOW_BYTES: u32 = 32;

/// Consecutive steps inside one window before a spin warning is printed
/// 打印自旋告警前，PC 连续停留在同一代码窗口内的步数
const SPIN_REPORT_STEPS: u64 = 1_000_000;

/// Maximum number of spin warnings per run / 每次运行最多打印的自旋告警条数
const SPIN_REPORT_LIMIT: usize = 4;

/// Steps retained for the post-mortem PC trace / 停机后回溯保留的步数
const TAIL_TRACE_STEPS: usize = 32;

/// Reason reported when the instruction budget runs out
/// 指令预算耗尽时报告的原因
const BUDGET_REASON: &str = "instruction budget exhausted / 指令预算耗尽";

/// CPU architecture selection for `run` / `run` 的 CPU 架构选择
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchArg {
    /// Arm Cortex-M33 / Arm Cortex-M33
    Arm,
    /// Hazard3 RISC-V / Hazard3 RISC-V
    ///
    /// `riscv` is accepted as an alias so the documented FV-1 command works.
    /// 接受 `riscv` 别名，使文档中的 FV-1 命令可直接运行。
    #[value(alias = "riscv")]
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
    /// Halt on undefined instructions instead of skipping / 遇到未定义指令即停机
    pub strict_decode: bool,
    /// Memory ranges written to disk after the run / 运行结束后写入磁盘的内存区间
    pub dumps: Vec<MemoryDump>,
}

/// One `--dump ADDR:LEN:PATH` request / 一条 `--dump ADDR:LEN:PATH` 请求
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryDump {
    /// First address to read / 起始地址
    pub address: u32,
    /// Number of bytes to read / 读取字节数
    pub length: u32,
    /// Destination file / 目标文件
    pub path: PathBuf,
}

impl MemoryDump {
    /// Parse `ADDR:LEN:PATH` with decimal or `0x`-prefixed hex numbers
    /// 解析 `ADDR:LEN:PATH`，数字支持十进制或 `0x` 十六进制前缀
    ///
    /// # Errors
    /// Returns a message naming the malformed field.
    /// 返回指出格式错误字段的消息。
    pub fn parse(spec: &str) -> Result<Self, String> {
        let mut parts = spec.splitn(3, ':');
        let address = parts
            .next()
            .ok_or_else(|| format!("missing address in `{spec}`"))?;
        let length = parts
            .next()
            .ok_or_else(|| format!("missing length in `{spec}`"))?;
        let path = parts
            .next()
            .ok_or_else(|| format!("missing path in `{spec}`"))?;
        if path.is_empty() {
            return Err(format!("empty path in `{spec}`"));
        }
        Ok(Self {
            address: parse_number(address).ok_or_else(|| format!("bad address `{address}`"))?,
            length: parse_number(length).ok_or_else(|| format!("bad length `{length}`"))?,
            path: PathBuf::from(path),
        })
    }
}

/// Parse a decimal or `0x`-prefixed hexadecimal number
/// 解析十进制或带 `0x` 前缀的十六进制数字
fn parse_number(text: &str) -> Option<u32> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).ok()
    } else {
        text.parse().ok()
    }
}

/// Construction options shared by `run`, `repl` and `perfbench`
/// `run`、`repl` 与 `perfbench` 共用的机器构造选项
#[derive(Debug, Clone, Copy)]
pub struct MachineOptions {
    /// Architecture override / 架构覆盖
    pub arch: Option<ArchArg>,
    /// Total instruction budget / 总指令预算
    pub max_cycles: u64,
    /// Halt on undefined instructions instead of skipping / 遇到未定义指令即停机
    pub strict_decode: bool,
}

impl Default for MachineOptions {
    fn default() -> Self {
        Self {
            arch: None,
            max_cycles: DEFAULT_RUN_MAX_CYCLES,
            strict_decode: false,
        }
    }
}

/// Architecture-tagged CPU handle owned by [`Machine`]
/// [`Machine`] 持有的带架构标记的 CPU 句柄
///
/// `Box<dyn CpuCore>` cannot reach architecture-specific entry points such as
/// `CortexM33Core::write_xpsr`, which the GDB adapter needs to restore the
/// program status register. Keeping the concrete cores in an enum preserves
/// those calls without changing the shared [`CpuCore`] trait.
/// `Box<dyn CpuCore>` 无法访问 `CortexM33Core::write_xpsr` 这类架构专有入口，
/// 而 GDB 适配器恢复程序状态寄存器时需要它。用枚举保存具体核心即可保留这些
/// 调用，无需修改共享的 [`CpuCore`] trait。
enum CoreHandle {
    /// Cortex-M33 core / Cortex-M33 核心
    Arm(Box<CortexM33Core>),
    /// Hazard3 RV32 core / Hazard3 RV32 核心
    RiscV(Box<Hazard3Core>),
}

/// Boot result of [`Machine::new`]: core 0, its boot vectors and core 1
/// [`Machine::new`] 的启动结果：core0、其启动向量与 core1
///
/// Core 1 exists only for Arm images and starts in `Reset`.
/// core1 仅对 Arm 映像存在，并以 `Reset` 启动。
type BootedCores = (CoreHandle, Option<(u32, u32)>, Option<CortexM33Core>);

/// Dispatch one [`CpuCore`] method to the active architecture variant
/// 将单个 [`CpuCore`] 方法分发到当前架构分支
macro_rules! core_dispatch {
    ($core:expr, $method:ident $(, $arg:expr)*) => {
        match $core {
            CoreHandle::Arm(core) => core.$method($($arg),*),
            CoreHandle::RiscV(core) => core.$method($($arg),*),
        }
    };
}

impl CpuCore for CoreHandle {
    fn cpu_id(&self) -> CpuId {
        core_dispatch!(self, cpu_id)
    }

    fn arch(&self) -> rp2350_cpu::CpuArch {
        core_dispatch!(self, arch)
    }

    fn init(&mut self, bus: &mut dyn AddressBus) -> Result<(), rp2350_core::error::CoreError> {
        core_dispatch!(self, init, bus)
    }

    fn run_cycles(&mut self, bus: &mut dyn AddressBus, cycles: u64) -> (u64, CpuState) {
        core_dispatch!(self, run_cycles, bus, cycles)
    }

    fn step(
        &mut self,
        bus: &mut dyn AddressBus,
    ) -> Result<CpuState, rp2350_core::error::CoreError> {
        core_dispatch!(self, step, bus)
    }

    fn signal_interrupt(&mut self, irq: u16) {
        core_dispatch!(self, signal_interrupt, irq);
    }

    fn servicing_irq(&self) -> Option<u16> {
        core_dispatch!(self, servicing_irq)
    }

    fn pc(&self) -> u32 {
        core_dispatch!(self, pc)
    }

    fn set_pc(&mut self, pc: u32) {
        core_dispatch!(self, set_pc, pc);
    }

    fn read_register(&self, index: u8) -> u32 {
        core_dispatch!(self, read_register, index)
    }

    fn write_register(&mut self, index: u8, value: u32) {
        core_dispatch!(self, write_register, index, value);
    }

    fn reset(&mut self) {
        core_dispatch!(self, reset);
    }

    fn description(&self) -> String {
        core_dispatch!(self, description)
    }

    fn system_write(&mut self, addr: u32, value: u32) {
        core_dispatch!(self, system_write, addr, value);
    }

    fn system_read(&mut self, addr: u32) -> Option<u32> {
        core_dispatch!(self, system_read, addr)
    }

    fn state(&self) -> CpuState {
        core_dispatch!(self, state)
    }

    fn xpsr(&self) -> u32 {
        core_dispatch!(self, xpsr)
    }

    fn csr_read(&self, addr: u32) -> u32 {
        core_dispatch!(self, csr_read, addr)
    }

    fn total_cycles(&self) -> u64 {
        core_dispatch!(self, total_cycles)
    }
}

/// A booted RP2350 machine that can be stepped by any frontend
/// 可被任意前端单步驱动的已启动 RP2350 机器
///
/// The machine owns the bus, synthetic boot ROM, CPU core, boot peripherals
/// and USB controller. `run` uses it with tracing; `repl` and `perfbench` use
/// the same object from a [`rp2350_usb_host::CdcLink`] cycle callback, so the
/// guest and the USB host bridge always share one timeline.
/// 机器持有总线、合成 boot ROM、CPU 核心、启动外设与 USB 控制器。`run` 使用
/// 它输出轨迹；`repl` 与 `perfbench` 从 [`rp2350_usb_host::CdcLink`] 的周期
/// 回调驱动同一个对象，因此客户机与 USB 主机桥共享同一条时间线。
pub struct Machine {
    /// Address bus with flash, SRAM and peripherals / 含 Flash、SRAM 与外设的地址总线
    pub bus: SimpleBus,
    /// Synthetic boot ROM and trap dispatcher / 合成 boot ROM 与陷阱分发器
    bootrom: Rp2350Bootrom,
    /// CPU core / CPU 核心
    core: CoreHandle,
    /// Cortex-M33 core 1, released through the SIO FIFO launch protocol
    /// 通过 SIO FIFO 启动协议释放的 Cortex-M33 core1
    core1: Option<CortexM33Core>,
    /// Why core 1 stopped, if it did / core1 的停机原因（若停机）
    core1_halt: Option<String>,
    /// Completed core-1 launches / 已完成的 core1 启动次数
    core1_launches: u64,
    /// Strict undefined-instruction mode, shared with core 1
    /// 未定义指令严格模式，与 core1 共享
    strict_decode: bool,
    /// Boot peripheral handles / 启动外设句柄
    peripherals: BootPeripherals,
    /// USB controller shared with the host bridge / 与主机桥共享的 USB 控制器
    usb: Controller,
    /// Whether boot ROM traps apply to this architecture / boot ROM 陷阱是否适用
    bootrom_applicable: bool,
    /// Image summary printed by `run` / `run` 打印的映像摘要
    image_description: String,
    /// Boot vectors for ARM images / ARM 映像的启动向量
    boot_vectors: Option<(u32, u32)>,
    /// Executed steps (treated as cycles) / 已执行步数（按周期计）
    steps: u64,
    /// Instruction budget / 指令预算
    budget: u64,
    /// Steps already reflected in the peripheral timebase / 已计入外设时基的步数
    advanced: u64,
    /// Steps at the last interrupt pump / 上次中断泵送时的步数
    pumped: u64,
    /// Steps at the last DMA pump / 上次 DMA 泵送时的步数
    dma_pumped: u64,
    /// Steps at the last PIO pump / 上次 PIO 泵送时的步数
    pio_pumped: u64,
    /// Cumulative DMA data units moved by the host pump / 宿主泵送累计搬运的 DMA 数据单元数
    dma_transfers: u64,
    /// Cumulative HSTX output words drained by the host pump / 宿主泵送累计排空的 HSTX 输出字数
    hstx_words: u64,
    /// Non-budget halt reason / 非预算原因的停机原因
    halt: Option<String>,
    /// Whether the peripheral timebase has been finalized / 外设时基是否已收尾
    finished: bool,
    /// Logged interrupt deliveries / 已记录的中断投递
    irqs: Vec<String>,
    /// Synchronous trap history (first entries) / 同步陷阱历史（前若干条）
    traps: Vec<String>,
    /// Last recorded synchronous `mcause` / 上次记录的同步 `mcause`
    last_trap_cause: u32,
    /// Buffered UART0 output / 已缓冲的 UART0 输出
    uart: Vec<u8>,
    /// Live notices (trace lines and spin warnings) / 实时通知（轨迹行与自旋告警）
    notices: Vec<String>,
    /// Start of the current spin window / 当前自旋窗口起点
    spin_base: u32,
    /// Consecutive steps inside the current spin window / 当前窗口内的连续步数
    spin_steps: u64,
    /// Number of spin warnings already printed / 已打印的自旋告警条数
    spin_reports: usize,
    /// Ring buffer of recent `(step, PC)` pairs for post-mortem traces
    /// 用于停机回溯的最近 `(步数, PC)` 环形缓冲
    recent: VecDeque<(u64, u32)>,
}

impl Machine {
    /// Build the bus, load firmware and boot the selected core
    /// 构建总线、加载固件并启动所选核心
    ///
    /// # Errors
    /// Returns an error when the firmware cannot be parsed or loaded, when a
    /// bus region cannot be registered, or when the core fails to boot.
    /// 固件无法解析或加载、总线区域注册失败，或核心启动失败时返回错误。
    pub fn new(bytes: &[u8], options: &MachineOptions) -> Result<Self> {
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
        // 0x1800_0000..0x1C00_0000 是 XIP 维护窗口，写入即请求缓存维护，数据被丢弃。
        // Flash 辅助代码用它清理 XIP SRAM 的缓存行；本模拟器没有缓存模型，
        // 因此这里只吸收写入、读出 0，而不是让它落到 NoDevice。
        // 0x1800_0000..0x1C00_0000 is the XIP maintenance window: writes request
        // cache maintenance and are discarded. Flash helpers use it to clean cache
        // lines covering XIP SRAM; this emulator has no cache model, so the window
        // absorbs writes and reads as zero instead of hitting NoDevice.
        bus.register_region(Box::new(XipMaintenanceRegion::new("xip-maintenance")))?;
        bus.register_region(Box::new(RamRegion::new("sram", SRAM_BASE, SRAM_SIZE)))?;
        let peripherals = BootPeripherals::mount(&mut bus)?;
        // USBCTRL 挂载后，DPSRAM（0x5010_0000）与寄存器窗口（0x5011_0000，含
        // XOR/SET/CLR 别名）由控制器建模，固件对 SIE_CTRL（0x5011_004C）的
        // 读-改-写不再落到 NoDevice。控制器句柄保留到机器生命周期结束，供
        // `repl` / `perfbench` 的主机桥驱动。
        // After mounting USBCTRL, the DPSRAM window at 0x5010_0000 and the register
        // window at 0x5011_0000 (with XOR/SET/CLR aliases) are modelled, so the
        // firmware read-modify-write of SIE_CTRL (0x5011_004C) no longer hits
        // NoDevice. The controller stays alive for the machine lifetime and is
        // driven by the host bridge in `repl` / `perfbench`.
        let usb = rp2350_usb::mount(&mut bus).context("USB mount failed / USB 挂载失败")?;

        let (arch, elf_entry, image_description) = load_image(bytes, &mut bus, options.arch)?;

        let (core, boot_vectors, core1): BootedCores = match arch {
            ArchArg::Arm => {
                let mut arm = CortexM33Core::new(CpuId::Core0);
                arm.set_strict_decode(options.strict_decode);
                // The RP2350 boot ROM programs SCB->VTOR to the flash vector
                // table before entering the image. The core caches VTOR, but
                // the PPB window has its own register file, so seed it here as
                // well; otherwise a guest read of 0xE000_ED08 sees zero and
                // copies an empty vector table into RAM (Pico SDK runtime_init).
                // RP2350 boot ROM 在进入映像前把 SCB->VTOR 指向 Flash 向量表。
                // 核心缓存了 VTOR，但 PPB 窗口有自己的寄存器组，因此这里也要
                // 同步写入；否则客户机读 0xE000_ED08 会得到 0，Pico SDK 的
                // runtime_init 会把空向量表复制到 RAM。
                let mut seed_cycles = 0u64;
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
                bus.write_u32(0xE000_ED08, FLASH_XIP_BASE, &mut seed_cycles)
                    .context("seed SCB_VTOR failed / 初始化 SCB_VTOR 失败")?;
                // core1 以 Reset 状态创建：SIO FIFO 启动协议完成前绝不取指。
                // Core 1 starts in Reset and never fetches before the SIO FIFO
                // launch protocol hands it vectors.
                let mut core1 = CortexM33Core::new(CpuId::Core1);
                core1.set_strict_decode(options.strict_decode);
                (CoreHandle::Arm(Box::new(arm)), vectors, Some(core1))
            }
            ArchArg::RiscV => {
                let mut riscv = Hazard3Core::new(CpuId::Core0);
                riscv
                    .init(&mut bus)
                    .context("core boot failed / 核心启动失败")?;
                // 同步安装合成 boot ROM 的致命异常向量：cause 2/5 进入宿主
                // 诊断桩，而不是客户机未填充的向量槽。
                // Install the synthetic boot ROM fatal-exception vectors so
                // causes 2/5 reach the host diagnostic stub instead of an
                // unfilled guest vector slot.
                riscv.set_fatal_trap_vector(bootrom.riscv_exception_vector());
                (CoreHandle::RiscV(Box::new(riscv)), None, None)
            }
        };

        let spin_base = core.pc();
        Ok(Self {
            bus,
            bootrom,
            core,
            core1,
            core1_halt: None,
            core1_launches: 0,
            strict_decode: options.strict_decode,
            peripherals,
            usb,
            // 两种架构都使用合成 boot ROM：Arm 走表查询桩，RISC-V 走
            // 0x7DFA 槽位暴露的 rom_func_lookup 桩。
            // Both architectures use the synthetic boot ROM: Arm goes through
            // the table lookup stub, RISC-V through the rom_func_lookup stub
            // exposed at the 0x7DFA slot.
            bootrom_applicable: true,
            image_description,
            boot_vectors,
            steps: 0,
            budget: options.max_cycles,
            advanced: 0,
            pumped: 0,
            dma_pumped: 0,
            pio_pumped: 0,
            dma_transfers: 0,
            hstx_words: 0,
            halt: None,
            finished: false,
            irqs: Vec::new(),
            traps: Vec::new(),
            // 复位后 mcause 为 0；用 0 作为初值可避免把复位值误记为陷阱。
            // mcause resets to 0; seeding with 0 avoids logging the reset value
            // as a trap.
            last_trap_cause: 0,
            uart: Vec::new(),
            notices: Vec::new(),
            spin_base,
            spin_steps: 0,
            spin_reports: 0,
            recent: VecDeque::with_capacity(TAIL_TRACE_STEPS),
        })
    }

    /// Human-readable image summary / 可读的映像摘要
    #[must_use]
    pub fn image_description(&self) -> &str {
        &self.image_description
    }

    /// Human-readable core description / 可读的核心描述
    #[must_use]
    pub fn core_description(&self) -> String {
        self.core.description()
    }

    /// Boot vectors for ARM images / ARM 映像的启动向量
    #[must_use]
    pub fn boot_vectors(&self) -> Option<(u32, u32)> {
        self.boot_vectors
    }

    /// Current program counter / 当前程序计数器
    #[must_use]
    pub fn pc(&self) -> u32 {
        self.core.pc()
    }

    /// Current stack pointer register / 当前栈指针寄存器
    #[must_use]
    pub fn sp(&self) -> u32 {
        self.core.read_register(13)
    }

    /// Executed steps (also used as simulated cycles) / 已执行步数（同时按周期计）
    #[must_use]
    pub fn steps(&self) -> u64 {
        self.steps
    }

    /// Current simulated cycle count / 当前模拟周期数
    #[must_use]
    pub fn now_cycles(&self) -> u64 {
        self.steps
    }

    /// Cumulative DMA data units moved by the host pump
    /// 宿主泵送累计搬运的 DMA 数据单元数
    ///
    /// Counts one unit per performed transfer (not per byte) and stays `0`
    /// until firmware enables a channel and the machine pumps it.
    /// 按已执行的传输计数（不是按字节），在固件使能通道并被机器泵送前保持 `0`。
    #[must_use]
    pub fn dma_transfers(&self) -> u64 {
        self.dma_transfers
    }

    /// One-line DMA diagnostic: `dma=<units> busy=<channel mask>`
    /// 单行 DMA 诊断：`dma=<单元数> busy=<通道掩码>`
    ///
    /// Intended for machine-state lines only; it never changes `repl --exec`
    /// stdout semantics because it is printed on the host diagnostics stream.
    /// 仅用于机器状态行；它打印在宿主诊断流上，因此不改变 `repl --exec` 的
    /// stdout 语义。
    #[must_use]
    pub fn dma_state(&self) -> String {
        format!(
            "dma={} busy={:#06X}",
            self.dma_transfers(),
            self.peripherals.dma().busy_mask()
        )
    }

    /// Cumulative HSTX output words drained by the host pump
    /// 宿主泵送累计排空的 HSTX 输出字数
    #[must_use]
    pub fn hstx_words(&self) -> u64 {
        self.hstx_words
    }

    /// One-line HSTX diagnostic / 单行 HSTX 诊断
    #[must_use]
    pub fn hstx_state(&self) -> String {
        let hstx = self.peripherals.hstx();
        format!(
            "hstx words={} underruns={} fifo={} enabled={} wof={} unknown={}",
            self.hstx_words(),
            hstx.output_underrun(),
            hstx.fifo_level(),
            hstx.enabled(),
            hstx.fifo_overrun(),
            hstx.unknown_commands()
        )
    }

    /// Busy state machines across all three PIO blocks / 三个 PIO 块中忙碌的状态机
    ///
    /// Bit `block * 4 + sm` is set when that state machine has `CTRL.SM_ENABLE`
    /// set. Host diagnostic only: it never prints on `repl --exec` stdout.
    /// 位 `block * 4 + sm` 表示该状态机已置位 `CTRL.SM_ENABLE`。仅供宿主诊断，
    /// 不会出现在 `repl --exec` 的 stdout。
    #[must_use]
    pub fn pio_busy_mask(&self) -> u16 {
        let mut mask = 0u16;
        for index in 0..rp2350_periph::pio::PIO_BLOCKS {
            if let Some(block) = self.peripherals.pio(index) {
                mask |= u16::from(block.enabled_mask()) << (index * 4);
            }
        }
        mask
    }

    /// Program counter of PIO state machine `sm` in `block` / `block` 中状态机 `sm` 的 PC
    ///
    /// Test/diagnostic hook: returns `None` for a block index above 2 or a
    /// machine index above 3.
    /// 测试与诊断接口：块序号大于 2 或状态机序号大于 3 时返回 `None`。
    #[cfg(test)]
    #[must_use]
    pub fn pio_pc(&self, block: usize, sm: usize) -> Option<u8> {
        self.peripherals.pio(block)?.pc(sm)
    }

    /// Elapsed simulated microseconds / 已模拟的微秒数
    #[must_use]
    pub fn time_us(&self) -> u64 {
        // 外设时基按 10_000 步批量推进，诊断时把尚未推进的步数也算进来。
        // The peripheral timebase advances in 10_000-step batches; include the
        // not-yet-advanced steps so diagnostics report the current time.
        let pending = self.steps.saturating_sub(self.advanced);
        let pending_us = pending / u64::from(rp2350_periph::boot::DEFAULT_CYCLES_PER_US);
        self.peripherals.time_us().saturating_add(pending_us)
    }

    /// Total instruction budget / 总指令预算
    #[must_use]
    pub fn budget(&self) -> u64 {
        self.budget
    }

    /// Whether the instruction budget has been exhausted / 指令预算是否已耗尽
    #[must_use]
    pub fn budget_exhausted(&self) -> bool {
        self.steps >= self.budget
    }

    /// Non-budget halt reason, if the machine stopped / 非预算原因的停机原因
    #[must_use]
    pub fn halt_reason(&self) -> Option<&str> {
        self.halt.as_deref()
    }

    /// Shared USB controller / 共享 USB 控制器
    #[must_use]
    pub fn usb(&self) -> &Controller {
        &self.usb
    }

    /// Read a core system-control register (test/diagnostic hook)
    /// 读取核心系统控制寄存器（测试与诊断接口）
    #[cfg(test)]
    #[must_use]
    pub fn system_register(&mut self, addr: u32) -> Option<u32> {
        self.core.system_read(addr)
    }

    /// Logged interrupt deliveries / 已记录的中断投递
    #[must_use]
    pub fn irqs(&self) -> &[String] {
        &self.irqs
    }

    /// Synchronous trap history / 同步陷阱历史
    #[must_use]
    pub fn traps(&self) -> &[String] {
        &self.traps
    }

    /// Take buffered UART0 output / 取走已缓冲的 UART0 输出
    pub fn take_uart_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.uart)
    }

    /// Take pending trace lines and spin warnings / 取走待输出的轨迹行与自旋告警
    pub fn take_notices(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notices)
    }

    /// Return the last executed steps with their first halfword
    /// 返回最近执行的若干步及其首个半字
    ///
    /// The ring buffer keeps only `(step, PC)` pairs; opcodes are read lazily
    /// from the bus when a post-mortem trace is requested, so the hot execution
    /// path does not pay for an extra bus access per instruction.
    /// 环形缓冲只保存 `(步数, PC)`；仅在请求回溯轨迹时才从总线读取操作码，
    /// 因此热路径不会因每条指令多一次总线访问而变慢。
    #[must_use]
    pub fn recent_steps(&mut self) -> Vec<(u64, u32, u16)> {
        let recent: Vec<(u64, u32)> = self.recent.iter().copied().collect();
        let mut cycles = 0u64;
        recent
            .into_iter()
            .map(|(step, pc)| {
                let opcode = self.bus.read_u16(pc, &mut cycles).unwrap_or(0xFFFF);
                (step, pc, opcode)
            })
            .collect()
    }

    /// Read a general-purpose register for diagnostics / 为诊断读取通用寄存器
    #[must_use]
    pub fn register(&self, index: u8) -> u32 {
        self.core.read_register(index)
    }

    /// Read a core register by index (0-15) / 按索引读取核心寄存器（0-15）
    #[must_use]
    pub fn read_register(&self, index: u8) -> u32 {
        self.core.read_register(index)
    }

    /// Write a core register by index (0-15) / 按索引写入核心寄存器（0-15）
    pub fn write_register(&mut self, index: u8, value: u32) {
        self.core.write_register(index, value);
    }

    /// Read the program status register (XPSR) / 读取程序状态寄存器（XPSR）
    ///
    /// Returns `0` for cores without an XPSR (RISC-V).
    /// 没有 XPSR 的核心（RISC-V）返回 `0`。
    #[must_use]
    pub fn xpsr(&self) -> u32 {
        self.core.xpsr()
    }

    /// Write the program status register (XPSR) / 写入程序状态寄存器（XPSR）
    ///
    /// Returns `false` when the active core has no XPSR (RISC-V), so a debugger
    /// can report the register as unsupported instead of silently dropping the
    /// write. / 当前核心没有 XPSR（RISC-V）时返回 `false`，调试器可据此报告该
    /// 寄存器不受支持，而不是静默丢弃写入。
    #[must_use]
    pub fn write_xpsr(&mut self, value: u32) -> bool {
        match &mut self.core {
            CoreHandle::Arm(core) => {
                core.write_xpsr(value);
                true
            }
            CoreHandle::RiscV(_) => false,
        }
    }

    /// Current core execution state / 当前核心执行状态
    #[must_use]
    pub fn cpu_state(&self) -> CpuState {
        self.core.state()
    }

    /// Core 1 execution state, or `None` for non-Arm architectures
    /// core1 执行状态；非 Arm 架构返回 `None`
    ///
    /// Core 1 starts in [`CpuState::Reset`] and only becomes `Running` after
    /// the SIO FIFO launch protocol hands it vectors.
    /// core1 以 [`CpuState::Reset`] 启动，只有在 SIO FIFO 启动协议交出向量后
    /// 才进入 `Running`。
    #[must_use]
    pub fn core1_state(&self) -> Option<CpuState> {
        self.core1.as_ref().map(CpuCore::state)
    }

    /// Core 1 program counter, or `None` / core1 程序计数器，无则 `None`
    #[must_use]
    pub fn core1_pc(&self) -> Option<u32> {
        self.core1.as_ref().map(CpuCore::pc)
    }

    /// Core 1 stack pointer, or `None` / core1 栈指针，无则 `None`
    #[must_use]
    pub fn core1_sp(&self) -> Option<u32> {
        self.core1.as_ref().map(|core| core.read_register(13))
    }

    /// Read one core-1 general-purpose register / 读取 core1 的一个通用寄存器
    #[cfg(test)]
    #[must_use]
    pub fn core1_register(&self, index: u8) -> Option<u32> {
        self.core1.as_ref().map(|core| core.read_register(index))
    }

    /// Number of completed core-1 launches / 已完成的 core1 启动次数
    #[must_use]
    pub fn core1_launch_count(&self) -> u64 {
        self.core1_launches
    }

    /// Why core 1 stopped, if it did / core1 的停机原因（若停机）
    ///
    /// Core-1 faults are recorded here instead of [`Self::halt_reason`] so a
    /// core-1 bus error never makes core 0 look like it crashed.
    /// core1 故障记录在这里而不是 [`Self::halt_reason`]，因此 core1 的总线错误
    /// 不会让 core0 看起来崩溃。
    #[must_use]
    pub fn core1_halt_reason(&self) -> Option<&str> {
        self.core1_halt.as_deref()
    }

    /// Read a core-1 system-control register (test/diagnostic hook)
    /// 读取 core1 系统控制寄存器（测试与诊断接口）
    #[cfg(test)]
    pub fn core1_system_register(&mut self, addr: u32) -> Option<u32> {
        self.core1.as_mut().and_then(|core| core.system_read(addr))
    }

    /// Dump a run of words from the bus, marking unmapped addresses
    /// 从总线转储一段连续字，对未映射地址做标记
    ///
    /// Returns `(address, value_or_marker)` pairs; unmapped reads report
    /// `0xFFFF_FFFF` so a missing region is visible at a glance.
    /// 返回 `(地址, 值或标记)`；未映射读返回 `0xFFFF_FFFF`，便于一眼看出
    /// 缺失区域。
    #[must_use]
    pub fn dump_memory(&mut self, addr: u32, words: usize) -> Vec<(u32, u32)> {
        let mut out = Vec::with_capacity(words);
        let mut cycles = 0u64;
        for i in 0..words {
            let a = addr.wrapping_add((i as u32) * 4);
            match self.bus.read_u32(a, &mut cycles) {
                Ok(value) => out.push((a, value)),
                Err(_) => out.push((a, 0xFFFF_FFFF)),
            }
        }
        out
    }

    /// Remember one executed PC for post-mortem diagnostics
    /// 记录一条已执行 PC 供停机后诊断
    fn record_step(&mut self, pc: u32) {
        if self.recent.len() == TAIL_TRACE_STEPS {
            self.recent.pop_front();
        }
        self.recent.push_back((self.steps, pc));
    }

    /// Record the first synchronous traps for post-mortem diagnostics
    /// 记录最先出现的同步陷阱供停机后诊断
    ///
    /// Interrupts (`mcause[31] == 1`) are excluded: they are normal traffic
    /// during enumeration. Only the first synchronous fault of each cause is
    /// kept, which is enough to locate a silent `ebreak` halt loop.
    /// 中断（`mcause[31] == 1`）被排除：枚举期间属于正常流量。每种原因只保留
    /// 首个同步故障，足以定位静默的 `ebreak` 停机循环。
    fn record_sync_trap(&mut self) {
        let cause = self.core.csr_read(0x342);
        if cause & 0x8000_0000 != 0 || cause == self.last_trap_cause {
            return;
        }
        self.last_trap_cause = cause;
        if self.traps.len() < IRQ_LOG_LIMIT {
            // 最近 PC 序列给出进入故障前的调用链，单看 mepc 无法区分「直接执行
            // ebreak」还是「某个断言函数跳到 ebreak」。
            // The recent PC sequence shows the call chain into the fault; mepc
            // alone cannot distinguish a direct ebreak from a panic helper.
            let history = self
                .recent
                .iter()
                .rev()
                .take(TAIL_TRACE_STEPS)
                .map(|(_, pc)| format!("{pc:#010X}"))
                .collect::<Vec<_>>()
                .join(" <- ");
            self.traps.push(format!(
                "[trap] step={} cause={cause:#X} mepc={:#010X} mtval={:#010X} hist={history}",
                self.steps,
                self.core.csr_read(0x341),
                self.core.csr_read(0x343)
            ));
        }
    }

    /// Execute one instruction on each runnable core
    /// 在每个可运行核心上各执行一条指令
    ///
    /// # Errors
    /// Propagates bus errors raised while dispatching a core-0 boot ROM trap.
    /// Core-1 faults are captured in [`Self::core1_halt_reason`] and never abort
    /// core 0.
    /// 传播 core0 boot ROM 陷阱分发期间的总线错误。core1 故障记录在
    /// [`Self::core1_halt_reason`] 中，绝不影响 core0。
    pub fn step(&mut self) -> Result<CpuState> {
        self.execute_step(None)
    }

    /// Execute one instruction with a `run`-style trace line
    /// 执行一条指令并生成 `run` 风格的轨迹行
    ///
    /// # Errors
    /// Same conditions as [`Self::step`]. / 错误条件与 [`Self::step`] 相同。
    pub fn trace_step(&mut self, index: usize) -> Result<CpuState> {
        self.execute_step(Some(index))
    }

    /// Advance the guest by at most `cycles` instructions
    /// 最多推进 `cycles` 条指令
    ///
    /// This is the callback used by [`rp2350_usb_host::CdcLink`]: it runs the
    /// same boot-ROM dispatch, interrupt pump, timebase advance and UART
    /// collection as the `run` path, and stops at the machine budget.
    /// 这是 [`rp2350_usb_host::CdcLink`] 使用的回调：执行与 `run` 路径相同的
    /// boot ROM 分发、中断泵送、时基推进与 UART 收集，并在机器预算处停止。
    pub fn run_cycles(&mut self, cycles: u64) {
        let target = self.steps.saturating_add(cycles).min(self.budget);
        while self.steps < target && self.halt.is_none() {
            match self.step() {
                Ok(CpuState::Running) => {}
                Ok(_) => break,
                Err(error) => {
                    if self.halt.is_none() {
                        self.halt = Some(error.to_string());
                    }
                    self.finalize();
                    break;
                }
            }
        }
    }

    /// Flush pending simulated time and UART bytes / 收尾模拟时间与 UART 字节
    pub fn finalize(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;
        self.advance_timebase();
        self.uart.extend(self.peripherals.take_uart_output());
    }

    fn execute_step(&mut self, trace: Option<usize>) -> Result<CpuState> {
        let state = self.execute_core0_step(trace)?;
        self.step_core1();
        Ok(state)
    }

    /// Advance core 1 by one instruction / 推进 core1 一条指令
    ///
    /// Handles the two handshakes that can change core 1's state (a `PSM`
    /// hard reset and a completed SIO FIFO launch) before stepping it. The SIO
    /// view is switched to core 1 around the instruction so `FIFO_WR`,
    /// `FIFO_RD`, the doorbells and `CPUID` follow core 1's direction, and PPB
    /// system writes are replayed into core 1 so its `VTOR`, `CPACR` and NVIC
    /// stay independent from core 0's. A core-1 fault only stops core 1.
    /// 在执行 core1 之前处理两种会改变其状态的握手（`PSM` 硬复位与已完成的 SIO
    /// FIFO 启动）。指令执行前后把 SIO 视图切到 core1，使 `FIFO_WR`、`FIFO_RD`、
    /// 门铃与 `CPUID` 遵循 core1 的方向；PPB 系统写入回放到 core1，使其
    /// `VTOR`、`CPACR` 与 NVIC 与 core0 相互独立。core1 故障只停止 core1。
    fn step_core1(&mut self) {
        if self.halt.is_some() || self.budget_exhausted() {
            return;
        }
        // PSM 置位 FRCE_OFF.PROC1：硬复位 core1 并重新启用 boot ROM 回显桩。
        // FRCE_OFF.PROC1 set: hard-reset core 1 and re-arm the boot-ROM echo stub.
        if self.peripherals.core1_reset_pending() && self.peripherals.take_core1_reset() {
            self.peripherals.reset_core1_boot();
            self.core1_halt = None;
            if self.core1.is_some() {
                let mut core1 = CortexM33Core::new(CpuId::Core1);
                core1.set_strict_decode(self.strict_decode);
                self.core1 = Some(core1);
            }
        }
        if self.peripherals.core1_launch_pending() {
            if let Some(launch) = self.peripherals.take_core1_launch() {
                self.boot_core1(launch);
            }
        }
        let Some(core1) = self.core1.as_mut() else {
            return;
        };
        if core1.state() != CpuState::Running {
            return;
        }
        self.peripherals.sio().select_core(CpuId::Core1);
        // core1 也走合成 boot ROM 的陷阱分发：`rom_table_lookup` 桩位于
        // 0x74，`runtime_init_per_core_bootrom_reset` 等每核初始化会调用它。
        // Core 1 also goes through the synthetic boot ROM trap dispatch: the
        // `rom_table_lookup` stub lives at 0x74 and the per-core runtime
        // initializers call it.
        let pc = core1.pc();
        let result: std::result::Result<CpuState, String> =
            if self.bootrom_applicable && self.bootrom.handles(pc) {
                match self.bootrom.dispatch(core1, &mut self.bus) {
                    Ok(Some(outcome)) => match outcome.halt {
                        Some(halt) => Err(format!("core1 bootrom halt: {halt}")),
                        None => Ok(CpuState::Running),
                    },
                    Ok(None) => core1.step(&mut self.bus).map_err(|error| error.to_string()),
                    Err(error) => Err(format!("core1: {error}")),
                }
            } else {
                core1.step(&mut self.bus).map_err(|error| error.to_string())
            };
        self.peripherals.sio().select_core(CpuId::Core0);
        // 按写入核心回放 PPB 系统写入：core1 的 CPACR/VTOR/NVIC 属于它自己。
        // Replay PPB system writes into the writing core: core 1's CPACR, VTOR
        // and NVIC belong to core 1.
        for (addr, value) in self.peripherals.take_system_writes() {
            core1.system_write(addr, value);
        }
        match result {
            Ok(CpuState::Running) => {}
            Ok(state) => {
                self.core1_halt = Some(format!("core1 entered state {state:?}"));
            }
            Err(reason) => {
                self.core1_halt = Some(reason);
            }
        }
    }

    /// Apply vectors from a completed FIFO launch / 应用已完成 FIFO 启动的向量
    ///
    /// The vector table is probed through the bus first, so a malformed `VTOR`
    /// stops core 1 instead of faulting inside a half-initialized core.
    /// 先通过总线探测向量表，使非法的 `VTOR` 只让 core1 停机，而不是在未完成
    /// 初始化时触发故障。
    fn boot_core1(&mut self, launch: Core1Launch) {
        let mut cycles = 0u64;
        if self.bus.read_u32(launch.vtor, &mut cycles).is_err()
            || self
                .bus
                .read_u32(launch.vtor.wrapping_add(4), &mut cycles)
                .is_err()
        {
            self.core1_halt = Some(format!("core1 vtor {:#010X} unreadable", launch.vtor));
            return;
        }
        if let Some(core1) = self.core1.as_mut() {
            core1.boot_at(launch.sp, launch.entry, launch.vtor);
        }
        if std::env::var_os("RP2350_TRACE_CORE1").is_some() {
            eprintln!(
                "[core1] launch vtor={:#010X} sp={:#010X} entry={:#010X}",
                launch.vtor, launch.sp, launch.entry
            );
        }
        self.core1_halt = None;
        self.core1_launches = self.core1_launches.saturating_add(1);
    }

    fn execute_core0_step(&mut self, trace: Option<usize>) -> Result<CpuState> {
        if self.halt.is_some() || self.budget_exhausted() {
            return Ok(CpuState::Halted);
        }
        let pc = self.core.pc();
        self.record_step(pc);
        if self.bootrom_applicable && self.bootrom.handles(pc) {
            if let Some(index) = trace {
                self.notices
                    .push(format!("[trace] {index:03} PC={pc:#010X} bootrom-trap"));
            }
            if let Some(outcome) = self.bootrom.dispatch(&mut self.core, &mut self.bus)? {
                self.steps += 1;
                self.record_sync_trap();
                // DMA 先于中断投递泵送，完成中断可在同一批次送达客户机。
                // DMA is pumped before IRQ delivery so a completion interrupt
                // is visible to the guest in the same batch.
                self.pump_dma_batch();
                self.pump_pio_batch();
                if let Some(halt) = outcome.halt {
                    self.halt = Some(format!("bootrom halt: {halt}"));
                    self.finalize();
                    return Ok(CpuState::Halted);
                }
                // 陷阱步与原 run 路径一致：立即泵送中断，不在此处推进时基。
                // Trap steps mirror the original run path: pump interrupts now and
                // do not advance the timebase here.
                pump(
                    &mut self.core,
                    self.core1.as_mut(),
                    &self.peripherals,
                    &self.usb,
                    &mut self.irqs,
                );
                return Ok(CpuState::Running);
            }
        }
        if let Some(index) = trace {
            let mut cycles = 0u64;
            let opcode = self.bus.read_u16(pc, &mut cycles).unwrap_or(0xFFFF);
            self.notices
                .push(format!("[trace] {index:03} PC={pc:#010X} op={opcode:#06X}"));
        }
        match self.core.step(&mut self.bus) {
            Ok(CpuState::Running) => {
                self.steps += 1;
                self.record_sync_trap();
                // 唯一 DMA 泵送点：普通步、轨迹步、boot ROM 陷阱步与 GDB
                // 单步都经过这里，调试器无法绕过 DMA 推进。
                // The single DMA pump point: plain, traced, boot ROM trap and
                // GDB single steps all pass here, so a debugger cannot bypass
                // DMA progress.
                self.pump_dma_batch();
                self.pump_pio_batch();
                if trace.is_some() {
                    // 轨迹步与原 run 路径一致：每步泵送中断，不做自旋统计或时基推进。
                    // Trace steps mirror the original run path: pump every step and
                    // skip spin accounting and timebase advancement.
                    pump(
                        &mut self.core,
                        self.core1.as_mut(),
                        &self.peripherals,
                        &self.usb,
                        &mut self.irqs,
                    );
                } else {
                    self.after_running_step();
                }
                Ok(CpuState::Running)
            }
            Ok(state) => {
                self.halt = Some(format!("core entered state {state:?}"));
                self.finalize();
                Ok(state)
            }
            Err(error) => {
                self.halt = Some(error.to_string());
                self.finalize();
                Ok(CpuState::Faulted)
            }
        }
    }

    fn after_running_step(&mut self) {
        // 紧凑自旋通常意味着固件在轮询一个永远不变的 MMIO 值（未建模设备静默
        // 读 0）或执行延时循环，会无声耗尽指令预算。PC 连续停在同一小窗口内
        // 达到阈值就告警一次，并给出窗口范围以便反汇编定位。
        // A tight spin usually means the firmware polls a register that never
        // changes (unmodelled devices read zero) or runs a delay loop, silently
        // burning the budget. Warn once when the PC stays inside one small window
        // and print the window so the spin point can be disassembled.
        let pc = self.core.pc();
        if pc.wrapping_sub(self.spin_base) < SPIN_WINDOW_BYTES {
            self.spin_steps += 1;
            if self.spin_steps == SPIN_REPORT_STEPS && self.spin_reports < SPIN_REPORT_LIMIT {
                self.spin_reports += 1;
                let registers = match self.core.arch() {
                    rp2350_cpu::CpuArch::RiscVHazard3 => format!(
                        "ra={:#010X} sp={:#010X} s0={:#010X} s1={:#010X} \
                         a0={:#010X} a1={:#010X} a2={:#010X} a3={:#010X} \
                         a4={:#010X} a5={:#010X}",
                        self.core.read_register(1),
                        self.core.read_register(2),
                        self.core.read_register(8),
                        self.core.read_register(9),
                        self.core.read_register(10),
                        self.core.read_register(11),
                        self.core.read_register(12),
                        self.core.read_register(13),
                        self.core.read_register(14),
                        self.core.read_register(15),
                    ),
                    rp2350_cpu::CpuArch::ArmCortexM33 => format!(
                        "r0={:#010X} r1={:#010X} r2={:#010X} r3={:#010X} sp={:#010X}",
                        self.core.read_register(0),
                        self.core.read_register(1),
                        self.core.read_register(2),
                        self.core.read_register(3),
                        self.core.read_register(13),
                    ),
                };
                self.notices.push(format!(
                    "[run] tight spin: PC stayed in {:#010X}..{:#010X} for {} steps, \
                     likely polling a value that never changes / 疑似轮询等待; {registers}",
                    self.spin_base,
                    self.spin_base + SPIN_WINDOW_BYTES,
                    self.spin_steps
                ));
            }
        } else {
            self.spin_base = pc;
            self.spin_steps = 0;
        }
        // 系统控制寄存器写入必须在下一条指令前生效：固件写 CPACR 后紧跟
        // `mrc p4`/VFP 指令，按 64 步批量回放会让核误判协处理器未使能并触发
        // NOCP UsageFault。中断线保持节流，避免电平中断被每一步重复投递。
        // System-control writes must land before the next instruction: firmware
        // writes CPACR immediately before `mrc p4`/VFP instructions, so a
        // 64-step batch replay makes the core raise a spurious NOCP UsageFault.
        // IRQ lines stay throttled to avoid re-asserting a level IRQ each step.
        replay_system_writes(&mut self.core, &self.peripherals);
        if self.steps - self.pumped >= PUMP_INTERVAL {
            self.peripherals.service_core1_peer();
            deliver_irqs(
                &mut self.core,
                self.core1.as_mut(),
                &self.peripherals,
                &self.usb,
                &mut self.irqs,
            );
            self.pumped = self.steps;
        }
        if self.steps - self.advanced >= ADVANCE_INTERVAL {
            self.advance_timebase();
        }
    }

    /// Run one throttled DMA batch before interrupt delivery
    /// 在中断投递前执行一个受节流的 DMA 批次
    ///
    /// Throttled to one call per [`PUMP_INTERVAL`] retired steps and capped at
    /// [`DMA_PUMP_TRANSFERS`] data units, so the CPU always executes at least
    /// that many instructions between batches. [`BootPeripherals::pump_dma`]
    /// republishes the PIO FIFO levels onto the DREQ array before running and
    /// re-samples them per unit for channels paced by a PIO selector, so a
    /// `TREQ_SEL` pointing at `PIOx_TXy`/`PIOx_RXy` only advances while the
    /// selected FIFO has room or data. The idle path costs a single `busy_mask`
    /// probe and never touches the bus or the PIO blocks.
    /// 按已退役步数每 [`PUMP_INTERVAL`] 步节流一次，每批最多
    /// [`DMA_PUMP_TRANSFERS`] 个数据单元，因此批次之间 CPU 至少执行同样多的
    /// 指令。[`BootPeripherals::pump_dma`] 在运行前把 PIO FIFO 电平重新发布到
    /// DREQ 数组，并对由 PIO 选择器节拍的通道逐单元重新采样，因此指向
    /// `PIOx_TXy`/`PIOx_RXy` 的 `TREQ_SEL` 只会在所选 FIFO 有空间或有数据时推进。
    /// 空闲路径只做一次 `busy_mask` 探测，不访问总线与 PIO 块。
    fn pump_dma_batch(&mut self) {
        if self.steps - self.dma_pumped < PUMP_INTERVAL {
            return;
        }
        // 先记节流点：即使当前空闲，下一次检查也推迟一整批，避免每步加锁。
        // Record the throttle point first: even when idle, the next probe waits
        // a full batch instead of locking on every step.
        self.dma_pumped = self.steps;
        // HSTX advances independently of DMA: drain the host queue every batch,
        // advance the expander by one batch of cycles and publish its FIFO-room
        // DREQ before arbitration.
        // HSTX 独立于 DMA 推进：每批排空宿主队列、推进一个批次周期，并在仲裁前
        // 发布 FIFO 空间 DREQ。
        self.peripherals.hstx_mut().advance(PUMP_INTERVAL);
        let drained = self.peripherals.hstx_mut().take_output_count();
        self.hstx_words = self.hstx_words.saturating_add(drained);
        self.peripherals.sync_hstx_dreq();
        if self.peripherals.dma().busy_mask() == 0 {
            return;
        }
        // bus 与外设是不同字段，可直接分开借用，无需 RefCell。
        // `bus` and the peripheral handle are distinct fields, so they borrow
        // separately without a RefCell.
        let moved = self.peripherals.pump_dma(&mut self.bus, DMA_PUMP_TRANSFERS);
        self.dma_transfers = self.dma_transfers.saturating_add(moved);
    }

    /// Advance the PIO state machines by the retired steps of this batch
    /// 按本批次退役步数推进 PIO 状态机
    ///
    /// Throttled to one call per [`PUMP_INTERVAL`] retired steps, exactly like
    /// the DMA pump. Every cycle handed to a block is one PIO clock, so the
    /// model assumes the PIO clock equals the CPU clock (one step = one PIO
    /// cycle). Each block checks `CTRL.SM_ENABLE` after one uncontended lock,
    /// so an unused PIO costs three lock acquires per batch and no per-machine
    /// work, and its interrupt lines are only republished when the `IRQ` flags
    /// change. Blocks are ticked before IRQ delivery, so a flag raised in this
    /// batch reaches the CPU in the same batch.
    /// 与 DMA 泵送一样按退役步数每 [`PUMP_INTERVAL`] 步节流一次。交给块的一个
    /// 周期就是一个 PIO 时钟，因此模型假定 PIO 时钟等于 CPU 时钟（一步 = 一个
    /// PIO 周期）。每个块在一次无竞争加锁后检查 `CTRL.SM_ENABLE`，未使用的 PIO
    /// 每批只花三次加锁、不做状态机工作，且只在 `IRQ` 标志变化时重新发布中断线。
    /// 块在中断投递之前推进，因此本批次置位的标志会在同一批次送达 CPU。
    fn pump_pio_batch(&mut self) {
        if self.steps - self.pio_pumped < PUMP_INTERVAL {
            return;
        }
        let elapsed = self.steps - self.pio_pumped;
        self.pio_pumped = self.steps;
        // 空闲快速路径：三个块都没有使能状态机时只做一次掩码探测就返回。
        // Idle fast path: with no enabled state machine in any of the three
        // blocks, one mask probe is the whole batch.
        let busy = self.pio_busy_mask();
        if busy == 0 {
            return;
        }
        for index in 0..rp2350_periph::pio::PIO_BLOCKS {
            if busy & (0x000F_u16 << (index * 4)) == 0 {
                continue;
            }
            if let Some(block) = self.peripherals.pio(index) {
                block.tick(elapsed);
            }
        }
    }

    fn advance_timebase(&mut self) {
        if self.steps > self.advanced {
            self.peripherals.advance(self.steps - self.advanced);
            self.advanced = self.steps;
            self.uart.extend(self.peripherals.take_uart_output());
        }
    }
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

    let machine_options = MachineOptions {
        arch: options.arch,
        max_cycles: options.max_cycles,
        strict_decode: options.strict_decode,
    };
    let mut machine = Machine::new(&bytes, &machine_options)?;

    let mut stdout = std::io::stdout();
    writeln!(stdout, "[run] {}", machine.image_description())?;
    writeln!(stdout, "[run] core: {}", machine.core_description())?;
    if let Some((sp, pc)) = machine.boot_vectors() {
        writeln!(stdout, "[run] boot vectors: SP={sp:#010X} PC={pc:#010X}")?;
    }
    writeln!(
        stdout,
        "[run] started: PC={:#010X} SP={:#010X}",
        machine.pc(),
        machine.sp()
    )?;

    for index in 0..options.trace_steps {
        let state = machine.trace_step(index)?;
        drain_notices(&mut machine, &mut stdout)?;
        if state != CpuState::Running {
            break;
        }
    }
    while machine.halt_reason().is_none() && !machine.budget_exhausted() {
        machine.step()?;
        drain_notices(&mut machine, &mut stdout)?;
    }
    machine.finalize();

    let reason = machine
        .halt_reason()
        .map_or_else(|| BUDGET_REASON.to_string(), str::to_owned);
    writeln!(stdout, "[run] halt: {reason}")?;
    for (step, pc, opcode) in machine.recent_steps() {
        writeln!(stdout, "[tail] {step:06} PC={pc:#010X} op={opcode:#06X}")?;
    }
    writeln!(
        stdout,
        "[run] executed {steps} steps, {us} us simulated, final PC={pc:#010X}",
        steps = machine.steps(),
        us = machine.time_us(),
        pc = machine.pc()
    )?;
    print_post_mortem(&mut machine, &mut stdout)?;
    write_dumps(&mut machine, &options.dumps)?;
    for dump in &options.dumps {
        writeln!(
            stdout,
            "[run] dump {:#010X}+{:#X} -> {}",
            dump.address,
            dump.length,
            dump.path.display()
        )?;
    }
    stdout.flush()?;
    Ok(())
}

/// Print the peripheral, register and UART diagnostics of a stopped machine
/// 打印停机机器的外设、寄存器与 UART 诊断
fn print_post_mortem(machine: &mut Machine, stdout: &mut impl Write) -> Result<()> {
    // DMA 统计只出现在 `run` 的宿主诊断行，`repl --exec` 的 stdout 不受影响。
    // The DMA counters only appear in `run`'s host diagnostics, so `repl --exec`
    // stdout semantics stay unchanged.
    writeln!(stdout, "[run] {}", machine.dma_state())?;
    writeln!(stdout, "[run] {}", machine.hstx_state())?;
    if let Some(state) = machine.core1_state() {
        writeln!(
            stdout,
            "[run] core1 state={state:?} pc={:#010X} sp={:#010X} launches={}",
            machine.core1_pc().unwrap_or(0),
            machine.core1_sp().unwrap_or(0),
            machine.core1_launch_count()
        )?;
        if let Some(reason) = machine.core1_halt_reason() {
            writeln!(stdout, "[run] core1 halt: {reason}")?;
        }
    }
    for index in 0..3 {
        if let Some(channel) = machine.peripherals.dma().channel_state(index) {
            writeln!(
                stdout,
                "[run] dma ch{index} rd={:#010X} wr={:#010X} cnt={} ctrl={:#010X} busy={} chain={} treq={}",
                channel.read_addr,
                channel.write_addr,
                channel.trans_count,
                channel.ctrl,
                channel.busy,
                channel.chain_to,
                channel.treq_sel
            )?;
        }
    }
    writeln!(stdout, "[run] {}", register_snapshot(machine))?;
    if !machine.irqs().is_empty() {
        writeln!(stdout, "[run] irq deliveries (first {IRQ_LOG_LIMIT}):")?;
        for line in machine.irqs() {
            writeln!(stdout, "{line}")?;
        }
    }
    let uart = machine.take_uart_output();
    writeln!(stdout, "[run] uart0: {} bytes", uart.len())?;
    if !uart.is_empty() {
        writeln!(stdout, "[run] uart0 output:")?;
        stdout.write_all(&uart)?;
        if !uart.ends_with(b"\n") {
            writeln!(stdout)?;
        }
    }
    Ok(())
}

/// Write every `--dump` range from the stopped machine / 写入停机机器的每条 `--dump` 区间
fn write_dumps(machine: &mut Machine, dumps: &[MemoryDump]) -> Result<()> {
    for dump in dumps {
        write_memory_dump(&mut machine.bus, dump).with_context(|| {
            format!("memory dump failed / 内存导出失败: {}", dump.path.display())
        })?;
    }
    Ok(())
}

/// Read `dump.length` bytes from the bus and write them to `dump.path`
/// 从总线读取 `dump.length` 字节并写入 `dump.path`
///
/// Reads go through the normal bus, so SRAM, flash and MMIO windows work; the
/// caller is responsible for choosing a side-effect-free range.
/// 读取走正常总线，因此 SRAM、Flash 与 MMIO 窗口都可用；调用方需选择无副作用的
/// 区间。
fn write_memory_dump(bus: &mut SimpleBus, dump: &MemoryDump) -> Result<()> {
    let mut bytes = Vec::with_capacity(dump.length as usize);
    let mut cycles = 0u64;
    for offset in 0..dump.length {
        let address = dump.address.wrapping_add(offset);
        bytes.push(bus.read_u8(address, &mut cycles)?);
    }
    std::fs::write(&dump.path, &bytes)
        .with_context(|| format!("cannot write {}", dump.path.display()))?;
    Ok(())
}

/// Format a one-line register snapshot for post-mortem diagnostics
/// 生成停机后寄存器快照的单行文本
///
/// A bad indirect branch is far easier to diagnose when the registers that
/// produced the target are visible. Some emulator faults (for example an
/// ARM-state `BLX` target) report the *branch target* as a "bus address", so
/// the register file is the only way to tell a memory fault from an INVSTATE.
/// 错误的间接跳转若能同时看到产生目标的寄存器就很容易定位。部分模拟器故障
/// （例如 `BLX` 跳到 ARM 态地址）会把「跳转目标」当作「总线地址」上报，
/// 只有寄存器现场才能区分内存故障与 INVSTATE。
#[must_use]
pub fn register_snapshot(machine: &Machine) -> String {
    let mut parts = Vec::with_capacity(18);
    match machine.core.arch() {
        rp2350_cpu::CpuArch::RiscVHazard3 => {
            for index in 0..16u8 {
                parts.push(format!(
                    "X{index}={:#010X}",
                    machine.core.read_register(index)
                ));
            }
            parts.push(format!("SP={:#010X}", machine.core.read_register(2)));
            parts.push(format!("RA={:#010X}", machine.core.read_register(1)));
            parts.push(format!("PC={:#010X}", machine.core.pc()));
            parts.push(format!("MTVEC={:#010X}", machine.core.csr_read(0x305)));
            parts.push(format!("MCAUSE={:#010X}", machine.core.csr_read(0x342)));
            parts.push(format!("MEPC={:#010X}", machine.core.csr_read(0x341)));
            parts.push(format!("MTVAL={:#010X}", machine.core.csr_read(0x343)));
        }
        rp2350_cpu::CpuArch::ArmCortexM33 => {
            for index in 0..13u8 {
                parts.push(format!(
                    "R{index}={:#010X}",
                    machine.core.read_register(index)
                ));
            }
            parts.push(format!("SP={:#010X}", machine.core.read_register(13)));
            parts.push(format!("LR={:#010X}", machine.core.read_register(14)));
            parts.push(format!("PC={:#010X}", machine.core.pc()));
        }
    }
    format!("[regs] {}", parts.join(" "))
}

/// Dump `count` words starting at `addr` as one diagnostic line
/// 从 `addr` 起转储 `count` 个字，输出为单行诊断
///
/// Unmapped words print as `????` instead of aborting, so the dump stays
/// useful when the fault address itself is bogus.
/// 未映射的字打印为 `????` 而不是中断转储，因此即使出错地址本身非法也仍可用。
fn memory_line(machine: &mut Machine, label: &str, addr: u32, count: usize) -> String {
    let mut words = Vec::with_capacity(count);
    for index in 0..count {
        let slot = addr.wrapping_add((index as u32) * 4);
        let mut cycles = 0u64;
        match machine.bus.read_u32(slot, &mut cycles) {
            Ok(value) => words.push(format!("{value:08X}")),
            Err(_) => words.push(String::from("????????")),
        }
    }
    format!("[mem] {label} {addr:#010X}: {}", words.join(" "))
}

/// Format the post-mortem PC history for frontends that stop on a fault
/// 为因故障停机的前端格式化停机前的 PC 轨迹
///
/// `repl` and `perfbench` do not print a step trace, so without this hook a
/// guest-side bus fault is only visible as a final PC. Showing the last
/// [`TAIL_TRACE_STEPS`] instructions makes the offending branch locatable, the
/// register snapshot shows how its operands were produced, and the memory dump
/// of `R4`/`SP` reveals corrupted structures.
/// `repl` 与 `perfbench` 不输出逐步轨迹，客户机总线故障只剩一个最终 PC；
/// 打印停机前 [`TAIL_TRACE_STEPS`] 条指令即可定位罪魁分支，寄存器快照说明
/// 操作数从何而来，`R4`/`SP` 的内存转储则暴露被破坏的结构体。
pub fn fault_tail_lines(machine: &mut Machine) -> Vec<String> {
    let (object, stack, object_label) = match machine.core.arch() {
        // RISC-V has no r4 frame pointer; s0 (x8) is the usual object base
        // saved across calls, and x2 is the stack pointer.
        // RISC-V 没有 r4 帧指针；跨调用保存的对象基址通常是 s0（x8），
        // 栈指针是 x2。
        rp2350_cpu::CpuArch::RiscVHazard3 => (
            machine.core.read_register(8),
            machine.core.read_register(2),
            "S0",
        ),
        rp2350_cpu::CpuArch::ArmCortexM33 => (
            machine.core.read_register(4),
            machine.core.read_register(13),
            "R4",
        ),
    };
    let mut lines = vec![register_snapshot(machine)];
    lines.push(memory_line(machine, object_label, object, 12));
    lines.push(memory_line(machine, "SP", stack, 12));
    // 若对象首字是 SRAM 指针，再转储它指向的锁/状态字。
    // If the object's first word is an SRAM pointer, dump the lock/state word
    // it points at.
    let mut cycles = 0u64;
    let pointer = machine
        .bus
        .read_u32(object, &mut cycles)
        .unwrap_or(0xFFFF_FFFF);
    let target = pointer & !3;
    if target & 0xFFF0_0000 == 0x2000_0000 {
        lines.push(memory_line(machine, "OBJ>", target, 8));
    }
    lines.extend(
        machine
            .recent_steps()
            .into_iter()
            .map(|(step, pc, opcode)| format!("[tail] {step:06} PC={pc:#010X} op={opcode:#06X}")),
    );
    lines
}

/// Write queued trace lines and spin warnings / 输出排队的轨迹行与自旋告警
fn drain_notices(machine: &mut Machine, stdout: &mut dyn Write) -> Result<()> {
    for notice in machine.take_notices() {
        writeln!(stdout, "{notice}")?;
    }
    Ok(())
}

/// Detect the format, load it and return its architecture and summary
/// 检测格式、加载映像并返回其架构与摘要
fn load_image(
    bytes: &[u8],
    bus: &mut SimpleBus,
    override_arch: Option<ArchArg>,
) -> Result<(ArchArg, Option<u32>, String)> {
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
        let description = format!(
            "uf2: family={family} blocks={}/{} flash={} sram={} range={:#010X}..{:#010X}",
            report.blocks_written,
            report.blocks_total,
            report.wrote_flash,
            report.wrote_sram,
            report.min_address,
            report.max_address
        );
        Ok((arch, None, description))
    } else {
        let image =
            FirmwareImage::from_elf_bytes(bytes).context("ELF parse failed / ELF 解析失败")?;
        image
            .load_into(bus)
            .context("ELF load failed / ELF 加载失败")?;
        let arch = override_arch.unwrap_or(ArchArg::Arm);
        let description = format!(
            "elf: entry={:#010X} segments={}",
            image.entry_point,
            image.segments.len()
        );
        Ok((arch, Some(image.entry_point), description))
    }
}

/// Replay PPB system writes and deliver asserted interrupt lines
/// 将 PPB 系统写入回放到核，并投递已断言的中断线
///
/// Used on boot ROM traps and traced steps, where the original `run` loop
/// pumped every step. The untraced hot path uses the two halves separately so
/// system writes take effect immediately while IRQ lines stay throttled.
/// 用于 boot ROM 陷阱与轨迹步（原 `run` 循环每步都泵送）。无轨迹的热路径分开
/// 调用两半：系统写入立即生效，中断线保持节流。
fn pump(
    core: &mut dyn CpuCore,
    core1: Option<&mut CortexM33Core>,
    peripherals: &BootPeripherals,
    usb: &Controller,
    irq_log: &mut Vec<String>,
) {
    peripherals.service_core1_peer();
    replay_system_writes(core, peripherals);
    deliver_irqs(core, core1, peripherals, usb, irq_log);
}

/// Replay PPB system writes into the core
/// 将 PPB 系统写入回放到核
fn replay_system_writes(core: &mut dyn CpuCore, peripherals: &BootPeripherals) {
    for (addr, value) in peripherals.take_system_writes() {
        core.system_write(addr, value);
    }
}

/// Deliver asserted interrupt lines to both cores / 向两个核心投递已断言的中断线
///
/// Every shared line reaches both NVICs and each core's enables decide whether
/// it is taken; `SIO_IRQ_FIFO` / `SIO_IRQ_BELL` are per-core endpoints and are
/// taken from [`BootPeripherals::core1_irq_mask`] for core 1. This is what lets
/// core 1 run HSTX's `DMA_IRQ_0` handler.
/// 所有共享线同时送达两个 NVIC，由各核心的使能位决定是否响应；
/// `SIO_IRQ_FIFO` / `SIO_IRQ_BELL` 是按核端点，core1 使用
/// [`BootPeripherals::core1_irq_mask`]。这正是 core1 能运行 HSTX
/// `DMA_IRQ_0` 处理程序的原因。
fn deliver_irqs(
    core: &mut dyn CpuCore,
    core1: Option<&mut CortexM33Core>,
    peripherals: &BootPeripherals,
    usb: &Controller,
    irq_log: &mut Vec<String>,
) {
    let mask = peripherals.irq_mask();
    for irq in 0..64u16 {
        if mask & (1u64 << irq) == 0 {
            continue;
        }
        core.signal_interrupt(irq);
        log_irq(irq_log, "", irq, core.pc());
    }
    // USB 控制器把 INTR & INTE 聚合到 INTS；固件清 INTS 后条件自然撤销。
    // The USB controller aggregates INTR & INTE into INTS; the condition
    // disappears when firmware clears INTS.
    if usb.irq_asserted() {
        core.signal_interrupt(USBCTRL_IRQ);
        log_irq(irq_log, "", USBCTRL_IRQ, core.pc());
    }
    let Some(core1) = core1 else {
        return;
    };
    let mask = peripherals.core1_irq_mask();
    for irq in 0..64u16 {
        if mask & (1u64 << irq) == 0 {
            continue;
        }
        core1.signal_interrupt(irq);
        log_irq(irq_log, "core1 ", irq, core1.pc());
    }
}

/// Record the latest interrupt deliveries in a bounded ring
/// 在有界环形缓冲中记录最近的中断投递
fn log_irq(irq_log: &mut Vec<String>, core: &str, irq: u16, pc: u32) {
    if irq_log.len() == IRQ_LOG_LIMIT {
        irq_log.remove(0);
    }
    irq_log.push(format!("[irq] {core}signal {irq} at PC={pc:#010X}"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{cpacr_write_elf, infinite_loop_elf};
    use rp2350_periph::dma::{
        CTRL_CHAIN_TO_SHIFT, CTRL_EN, CTRL_INCR_READ, CTRL_INCR_WRITE, CTRL_TREQ_SEL_SHIFT,
        TREQ_PERMANENT,
    };
    use rp2350_periph::map::DMA_BASE;

    /// `SCB_CPACR` absolute address / `SCB_CPACR` 绝对地址
    const SCB_CPACR: u32 = 0xE000_ED88;

    /// Per-channel register stride / 每通道寄存器步长
    const CHANNEL_STRIDE: u32 = 0x40;

    /// `CTRL_TRIG.DATA_SIZE` value for word transfers / 字传输的 `DATA_SIZE` 值
    const DATA_SIZE_WORD: u32 = 2;

    /// Program one word-mode RAM-to-RAM channel through the guest bus
    /// 通过客户机总线编程一个字宽 RAM→RAM 通道
    fn program_channel(
        machine: &mut Machine,
        channel: u32,
        src: u32,
        dst: u32,
        count: u32,
        chain_to: u8,
    ) {
        let base = DMA_BASE + channel * CHANNEL_STRIDE;
        write_word(machine, base, src);
        write_word(machine, base + 0x04, dst);
        write_word(machine, base + 0x08, count);
        let ctrl = CTRL_EN
            | (DATA_SIZE_WORD << rp2350_periph::dma::CTRL_DATA_SIZE_SHIFT)
            | CTRL_INCR_READ
            | CTRL_INCR_WRITE
            | (u32::from(TREQ_PERMANENT) << CTRL_TREQ_SEL_SHIFT)
            | (u32::from(chain_to) << CTRL_CHAIN_TO_SHIFT);
        write_word(machine, base + 0x0C, ctrl);
    }

    /// Write one word through the machine bus / 通过机器总线写入一个字
    fn write_word(machine: &mut Machine, addr: u32, value: u32) {
        let mut cycles = 0u64;
        machine
            .bus
            .write_u32(addr, value, &mut cycles)
            .expect("bus write");
    }

    /// Read one word through the machine bus / 通过机器总线读取一个字
    fn read_word(machine: &mut Machine, addr: u32) -> u32 {
        let mut cycles = 0u64;
        machine.bus.read_u32(addr, &mut cycles).expect("bus read")
    }

    /// Build a machine with a fixed Arm budget / 构建带固定 Arm 预算的机器
    fn machine_with_budget(max_cycles: u64) -> Machine {
        let elf = infinite_loop_elf();
        let options = MachineOptions {
            arch: Some(ArchArg::Arm),
            max_cycles,
            strict_decode: false,
        };
        Machine::new(&elf, &options).expect("machine")
    }

    #[test]
    fn dma_ram_to_ram_channel_completes_under_the_machine() {
        // 固件只写 DMA 寄存器，传输应由机器主循环自动推进。
        // Firmware only programs the DMA registers; the machine main loop must
        // advance the transfer by itself.
        let mut machine = machine_with_budget(1_000);
        let src = SRAM_BASE + 0x1000;
        let dst = SRAM_BASE + 0x2000;
        let words = [0x1111_1111u32, 0x2222_2222, 0x3333_3333, 0x4444_4444];
        for (index, word) in words.iter().enumerate() {
            write_word(&mut machine, src + index as u32 * 4, *word);
        }
        program_channel(&mut machine, 0, src, dst, 4, 0);

        machine.run_cycles(200);

        assert_eq!(machine.dma_transfers(), 4);
        for (index, word) in words.iter().enumerate() {
            assert_eq!(read_word(&mut machine, dst + index as u32 * 4), *word);
        }
        let state = machine.peripherals.dma().channel_state(0).expect("ch0");
        assert_eq!(state.trans_count, 0);
        assert!(!state.busy);
    }

    #[test]
    fn a_busy_dma_channel_does_not_stall_the_cpu() {
        // 长传输期间 CPU 必须继续执行，DMA 只在每批预算内推进。
        // The CPU must keep executing during a long transfer; DMA only advances
        // within its per-batch budget.
        let mut machine = machine_with_budget(1_000);
        program_channel(
            &mut machine,
            0,
            SRAM_BASE + 0x1000,
            SRAM_BASE + 0x4000,
            1_000,
            0,
        );

        machine.run_cycles(300);

        assert_eq!(machine.steps(), 300);
        assert!(machine.dma_transfers() > 0);
        assert!(machine.dma_transfers() < 1_000);
        assert!(
            machine
                .peripherals
                .dma()
                .channel_state(0)
                .expect("ch0")
                .busy
        );
    }

    #[test]
    fn the_dma_batch_budget_is_bounded() {
        // 一个批次最多 16 个单元：不会一次搬完整块而长时间不返回。
        // One batch moves at most 16 units, so a large block never blocks the
        // main loop for long.
        let mut machine = machine_with_budget(1_000);
        program_channel(
            &mut machine,
            0,
            SRAM_BASE + 0x1000,
            SRAM_BASE + 0x4000,
            1_000,
            0,
        );

        machine.run_cycles(PUMP_INTERVAL);
        assert_eq!(machine.dma_transfers(), DMA_PUMP_TRANSFERS);
        assert_eq!(
            machine
                .peripherals
                .dma()
                .channel_state(0)
                .expect("ch0")
                .trans_count,
            1_000 - DMA_PUMP_TRANSFERS as u32
        );

        machine.run_cycles(PUMP_INTERVAL);
        assert_eq!(machine.dma_transfers(), 2 * DMA_PUMP_TRANSFERS);
    }

    #[test]
    fn idle_dma_reports_zero_transfers() {
        // 未编程任何通道时泵送必须完全跳过，步进行为保持不变。
        // With no channel programmed the pump must be a no-op and stepping must
        // keep its historical behaviour.
        let mut machine = machine_with_budget(5);
        machine.run_cycles(5);

        assert_eq!(machine.steps(), 5);
        assert!(machine.budget_exhausted());
        assert_eq!(machine.dma_transfers(), 0);
        assert_eq!(machine.peripherals.dma().busy_mask(), 0);
        assert_eq!(machine.dma_state(), "dma=0 busy=0x0000");
    }

    #[test]
    fn chained_channels_complete_under_the_machine() {
        // 链式通道：ch1 先被配置并中止，ch0 完成后由 CHAIN_TO 启动 ch1。
        // Chained channels: ch1 is configured then aborted; completing ch0
        // starts ch1 through CHAIN_TO.
        let mut machine = machine_with_budget(1_000);
        let src0 = SRAM_BASE + 0x1000;
        let dst0 = SRAM_BASE + 0x2000;
        let src1 = SRAM_BASE + 0x3000;
        let dst1 = SRAM_BASE + 0x4000;
        for index in 0..4u32 {
            write_word(&mut machine, src0 + index * 4, 0xA000_0000 | index);
        }
        for index in 0..2u32 {
            write_word(&mut machine, src1 + index * 4, 0xB000_0000 | index);
        }
        program_channel(&mut machine, 1, src1, dst1, 2, 1);
        // CHAN_ABORT 只清 BUSY，保留 EN 与计数，链式启动即可重新开跑。
        // CHAN_ABORT only clears BUSY, keeping EN and the count, so the chain
        // can restart it.
        write_word(&mut machine, DMA_BASE + 0x464, 1 << 1);
        program_channel(&mut machine, 0, src0, dst0, 4, 1);

        machine.run_cycles(200);

        assert_eq!(machine.dma_transfers(), 6);
        for index in 0..4u32 {
            assert_eq!(
                read_word(&mut machine, dst0 + index * 4),
                0xA000_0000 | index
            );
        }
        for index in 0..2u32 {
            assert_eq!(
                read_word(&mut machine, dst1 + index * 4),
                0xB000_0000 | index
            );
        }
        assert_eq!(machine.peripherals.dma().busy_mask(), 0);
    }

    #[test]
    fn traced_steps_advance_the_dma_pump() {
        // `run` 的轨迹前缀使用 trace_step；该路径也必须泵送 DMA。
        // The `run` trace prefix uses trace_step; that path must pump DMA too.
        let mut machine = machine_with_budget(1_000);
        program_channel(
            &mut machine,
            0,
            SRAM_BASE + 0x1000,
            SRAM_BASE + 0x2000,
            4,
            0,
        );

        for index in 0..PUMP_INTERVAL as usize {
            machine.trace_step(index).expect("trace step");
        }

        assert_eq!(machine.dma_transfers(), 4);
        let _ = machine.take_notices();
    }

    #[test]
    fn gdb_single_steps_also_pump_dma() {
        // GDB `resume` 走 `step`；单步不能绕过 DMA 推进。
        // GDB `resume` uses `step`; single stepping must not bypass DMA.
        let mut machine = machine_with_budget(1_000);
        program_channel(
            &mut machine,
            0,
            SRAM_BASE + 0x1000,
            SRAM_BASE + 0x2000,
            4,
            0,
        );

        for _ in 0..PUMP_INTERVAL {
            machine.step().expect("step");
        }

        assert_eq!(machine.dma_transfers(), 4);
    }

    /// PIO `CTRL` offset / PIO `CTRL` 偏移
    const PIO_CTRL: u32 = 0x000;

    /// PIO `IRQ` offset (write-one-to-clear) / PIO `IRQ` 偏移（写 1 清零）
    const PIO_IRQ: u32 = 0x030;

    /// PIO `INSTR_MEM0` offset / PIO `INSTR_MEM0` 偏移
    const PIO_INSTR_MEM0: u32 = 0x048;

    /// PIO `TXF0` offset / PIO `TXF0` 偏移
    const PIO_TXF0: u32 = 0x010;

    /// `SM0_EXECCTRL` offset / `SM0_EXECCTRL` 偏移
    const PIO_SM0_EXECCTRL: u32 = 0x0CC;

    /// `MOV Y, Y` no-op instruction / `MOV Y, Y` 空操作指令
    const PIO_NOP: u32 = 0xA042;

    /// `SET pins, data` instruction / `SET pins, data` 指令
    fn pio_set_pins(data: u32) -> u32 {
        0xE000 | (data & 0x1F)
    }

    /// `IRQ set, field` instruction / `IRQ set, field` 指令
    fn pio_irq_set(field: u32) -> u32 {
        0xC000 | (field & 0x1F)
    }

    #[test]
    fn pio_state_machines_tick_once_per_pump_batch() {
        // 10 条 NOP + WRAP_TOP = 9：PC 等于已执行拍数对 10 取模，因此 64 拍后
        // 为 4、128 拍后为 8，可区分“每 64 步 tick 一次”与“未 tick”。
        // Ten NOPs with WRAP_TOP = 9 make the PC the executed-cycle count mod
        // 10, so 64 cycles land on 4 and 128 on 8: the value distinguishes the
        // 64-step pump from no pump at all.
        let mut machine = machine_with_budget(1_000);
        let base = rp2350_periph::map::PIO0_BASE;
        write_word(&mut machine, base + PIO_INSTR_MEM0, pio_set_pins(1));
        for slot in 1..10 {
            write_word(&mut machine, base + PIO_INSTR_MEM0 + slot * 4, PIO_NOP);
        }
        write_word(&mut machine, base + PIO_SM0_EXECCTRL, 9 << 12);
        write_word(&mut machine, base + PIO_CTRL, 1);
        assert_eq!(machine.pio_busy_mask(), 0b0001);
        assert_eq!(machine.peripherals.pio(0).expect("pio0").output_levels(), 0);

        machine.run_cycles(PUMP_INTERVAL - 1);
        assert_eq!(machine.pio_pc(0, 0), Some(0), "no tick before the batch");
        assert_eq!(
            machine.peripherals.pio(0).expect("pio0").output_levels(),
            0,
            "no pin activity before the batch"
        );

        machine.run_cycles(1);
        assert_eq!(machine.pio_pc(0, 0), Some(4), "one batch of 64 cycles");
        assert_eq!(
            machine.peripherals.pio(0).expect("pio0").output_levels() & 1,
            1,
            "SET pins ran inside the batch"
        );

        machine.run_cycles(1);
        assert_eq!(machine.pio_pc(0, 0), Some(4), "one batch of 64 cycles");

        machine.run_cycles(PUMP_INTERVAL);
        assert_eq!(machine.pio_pc(0, 0), Some(8), "second batch");

        // 未使能的块不推进：SM1 始终保持 PC 0。
        // A disabled machine never advances: SM1 stays at PC 0.
        assert_eq!(machine.pio_pc(0, 1), Some(0));
    }

    #[test]
    fn pio_irq_flags_reach_the_shared_irq_mask() {
        // `IRQ set` 在泵送批次内执行，标志必须出现在共享中断掩码中；客户机
        // 写 1 清零后立即释放。
        // `IRQ set` runs inside a pump batch, so the flag must appear in the
        // shared IRQ mask and the guest's write-one-to-clear must release it.
        let mut machine = machine_with_budget(1_000);
        let base = rp2350_periph::map::PIO0_BASE;
        write_word(&mut machine, base + PIO_INSTR_MEM0, pio_irq_set(2));
        write_word(&mut machine, base + PIO_CTRL, 1);
        let line = 1u64 << rp2350_periph::irq::PIO0_IRQ_0;
        assert_eq!(machine.peripherals.irq_mask() & line, 0);

        machine.run_cycles(PUMP_INTERVAL);
        assert_ne!(machine.peripherals.irq_mask() & line, 0, "flag 2 asserted");

        write_word(&mut machine, base + PIO_IRQ, 1 << 2);
        assert_eq!(machine.peripherals.irq_mask() & line, 0, "W1C released");
        assert_eq!(machine.peripherals.pio(0).expect("pio0").irq_flags(), 0);
    }

    #[test]
    fn pio_tx_dreq_paces_the_dma_under_the_machine() {
        // PIO0_TX0 节拍的 DMA 只能填满 4 深度 FIFO，PIO 取走后继续推进。
        // A DMA channel paced by PIO0_TX0 can only fill the 4-deep FIFO; it
        // makes progress again after the PIO drains it.
        let mut machine = machine_with_budget(4_000);
        let src = SRAM_BASE + 0x1000;
        let pio_base = rp2350_periph::map::PIO0_BASE;
        for index in 0..6u32 {
            write_word(&mut machine, src + index * 4, 0x7000_0000 | index);
        }
        let ctrl = CTRL_EN
            | (DATA_SIZE_WORD << rp2350_periph::dma::CTRL_DATA_SIZE_SHIFT)
            | CTRL_INCR_READ
            | (u32::from(rp2350_periph::dreq::PIO0_TX0) << CTRL_TREQ_SEL_SHIFT);
        write_word(&mut machine, DMA_BASE, src);
        write_word(&mut machine, DMA_BASE + 0x04, pio_base + PIO_TXF0);
        write_word(&mut machine, DMA_BASE + 0x08, 6);
        write_word(&mut machine, DMA_BASE + 0x0C, ctrl);

        // 第一批：SM 关闭，DMA 填满 FIFO 后因 DREQ 变低而停止。
        // First batch: with the SM disabled the DMA fills the FIFO and stops
        // when the DREQ drops.
        machine.run_cycles(PUMP_INTERVAL);
        assert_eq!(machine.dma_transfers(), 4);
        assert_eq!(
            machine.peripherals.pio(0).expect("pio0").tx_fifo_full(0),
            Some(true)
        );
        assert_eq!(
            machine
                .peripherals
                .dma()
                .channel_state(0)
                .expect("ch0")
                .trans_count,
            2
        );
        machine.run_cycles(PUMP_INTERVAL);
        assert_eq!(machine.dma_transfers(), 4, "full FIFO gates the channel");

        // 启用 `pull block` 循环：PIO 取走两批数据后阻塞在空 FIFO 上。
        // Enable a `pull block` loop: the PIO drains both batches and finally
        // stalls on the empty FIFO.
        write_word(&mut machine, pio_base + PIO_INSTR_MEM0, 0x80A0);
        write_word(&mut machine, pio_base + PIO_INSTR_MEM0 + 4, 0x0000);
        write_word(&mut machine, pio_base + PIO_CTRL, 1);
        machine.run_cycles(PUMP_INTERVAL);
        machine.run_cycles(PUMP_INTERVAL);

        assert_eq!(machine.dma_transfers(), 6);
        let state = machine.peripherals.dma().channel_state(0).expect("ch0");
        assert_eq!(state.trans_count, 0);
        assert!(!state.busy);
        let pio0 = machine.peripherals.pio(0).expect("pio0");
        assert_eq!(pio0.tx_fifo_empty(0), Some(true));
        assert_ne!(pio0.stalled_mask() & 1, 0, "PULL waits on the empty FIFO");
        machine.peripherals.sync_pio_dreqs();
        assert_eq!(
            machine
                .peripherals
                .dma()
                .dreq(rp2350_periph::dreq::PIO0_TX0),
            Some(true)
        );
    }

    #[test]
    fn unused_pio_blocks_do_not_change_stepping() {
        // 没有任何 PIO 使能时，泵送只做一次状态探测，步数与 PC 行为不变。
        // With no PIO enabled the pump only probes state: step count and PC
        // behaviour stay unchanged.
        let mut machine = machine_with_budget(200);
        machine.run_cycles(200);
        assert_eq!(machine.steps(), 200);
        assert_eq!(machine.pio_busy_mask(), 0);
        assert_eq!(machine.pio_pc(0, 0), Some(0));
        assert_eq!(machine.peripherals.irq_mask(), 0);
    }

    #[test]
    fn memory_dump_specs_parse_decimal_and_hex() {
        let dump = MemoryDump::parse("0x20063DF4:0x12C00:fb.bin").expect("spec");
        assert_eq!(dump.address, 0x2006_3DF4);
        assert_eq!(dump.length, 0x1_2C00);
        assert_eq!(dump.path, PathBuf::from("fb.bin"));

        let plain = MemoryDump::parse("537000000:1024:out.bin").expect("decimal");
        assert_eq!(plain.address, 537_000_000);
        assert_eq!(plain.length, 1024);

        assert!(MemoryDump::parse("0x2000").is_err(), "missing fields");
        assert!(MemoryDump::parse("zz:1:x").is_err(), "bad address");
        assert!(MemoryDump::parse("1:zz:x").is_err(), "bad length");
        assert!(MemoryDump::parse("1:2:").is_err(), "empty path");
    }

    #[test]
    fn machine_boots_and_stops_at_the_budget() {
        let elf = infinite_loop_elf();
        let options = MachineOptions {
            arch: Some(ArchArg::Arm),
            max_cycles: 5,
            strict_decode: false,
        };
        let mut machine = Machine::new(&elf, &options).expect("machine");
        assert_eq!(
            machine.boot_vectors(),
            Some((SRAM_BASE + SRAM_SIZE as u32, 0x1000_0000))
        );
        assert_eq!(machine.pc(), 0x1000_0000);
        // The PPB window must expose the same VTOR that boot_at programmed in
        // the core; Pico SDK's runtime_init copies the RAM vector table from it.
        // PPB 窗口必须暴露与 core 相同的 VTOR；Pico SDK runtime_init 依赖它
        // 复制 RAM 向量表。
        let mut cycles = 0u64;
        assert_eq!(
            machine
                .bus
                .read_u32(0xE000_ED08, &mut cycles)
                .expect("VTOR readable"),
            0x1000_0000
        );

        machine.run_cycles(3);
        assert_eq!(machine.steps(), 3);
        assert!(!machine.budget_exhausted());

        machine.run_cycles(100);
        assert_eq!(machine.steps(), 5);
        assert!(machine.budget_exhausted());
        assert_eq!(machine.halt_reason(), None);

        machine.run_cycles(100);
        assert_eq!(machine.steps(), 5);
        assert_eq!(machine.pc(), 0x1000_0000);
        assert_eq!(machine.time_us(), 0);
    }

    #[test]
    fn trace_step_reports_the_branch_opcode() {
        let elf = infinite_loop_elf();
        let options = MachineOptions {
            arch: Some(ArchArg::Arm),
            max_cycles: 10,
            strict_decode: false,
        };
        let mut machine = Machine::new(&elf, &options).expect("machine");
        let state = machine.trace_step(0).expect("trace step");
        assert_eq!(state, CpuState::Running);
        let notices = machine.take_notices();
        assert_eq!(notices.len(), 1);
        assert!(notices[0].starts_with("[trace] 000 PC=0x10000000 op=0xE7FE"));
    }

    #[test]
    fn recent_steps_keep_a_bounded_pc_history() {
        let elf = infinite_loop_elf();
        let options = MachineOptions {
            arch: Some(ArchArg::Arm),
            max_cycles: 40,
            strict_decode: false,
        };
        let mut machine = Machine::new(&elf, &options).expect("machine");
        machine.run_cycles(40);
        let tail = machine.recent_steps();
        // 预算小于回溯窗口时，缓冲只保留实际执行过的步数。
        // When the budget is smaller than the trace window, the ring buffer
        // only holds the steps that actually ran.
        let expected = usize::try_from(machine.steps())
            .unwrap()
            .min(TAIL_TRACE_STEPS);
        assert_eq!(tail.len(), expected);
        assert_eq!(tail[0].0, machine.steps() - expected as u64);
        assert!(
            tail.iter()
                .all(|&(_, pc, opcode)| pc == 0x1000_0000 && opcode == 0xE7FE)
        );
    }

    #[test]
    fn ppb_writes_reach_the_core_before_the_next_instruction() {
        // 固件在写 CPACR 后紧接协处理器指令；若系统写入按 64 步批量回放，
        // 这里第 3 步的写入要到第 64 步才生效，核内 CPACR 仍是上电默认值。
        // Firmware writes CPACR right before a coprocessor instruction. With a
        // 64-step batch replay, the step-3 write would not land until step 64
        // and the in-core CPACR would still hold its power-on default.
        let elf = cpacr_write_elf();
        let options = MachineOptions {
            arch: Some(ArchArg::Arm),
            max_cycles: 8,
            strict_decode: false,
        };
        let mut machine = Machine::new(&elf, &options).expect("machine");
        assert_eq!(
            machine.system_register(SCB_CPACR),
            Some(0x00F0_0000),
            "power-on CPACR must model the boot ROM's FPU enable / 上电 CPACR 应复现 boot ROM 的 FPU 使能"
        );

        machine.run_cycles(4);
        assert_eq!(machine.steps(), 4);
        assert_eq!(machine.halt_reason(), None);
        assert_eq!(
            machine.system_register(SCB_CPACR),
            Some(0x0030_0303),
            "CPACR write must be replayed on the very next step / CPACR 写入必须在下一步前回放"
        );
    }

    #[test]
    fn sram_top_is_mapped_for_exception_stack_frames() {
        // MicroPython 把 MSP 放在 520 KB SRAM 顶端，软复位/异常压栈会访问
        // 顶端下方几百字节。若某个窗口抢先覆盖或 SRAM 尺寸算错，固件会在
        // raw REPL 握手阶段莫名报 NoDevice。
        // MicroPython parks MSP at the top of the 520 KB SRAM, so soft resets
        // and exception stacking touch the few hundred bytes below the top.
        // A clobbering window or a wrong SRAM size would surface as a spurious
        // NoDevice during the raw REPL handshake.
        use rp2350_core::bus::AddressBus;

        let elf = infinite_loop_elf();
        let machine = Machine::new(&elf, &MachineOptions::default()).expect("machine");
        let mut bus = machine.bus;
        let top = SRAM_BASE + SRAM_SIZE as u32;
        for addr in [top - 0x108, top - 0x20, top - 4] {
            let region = bus
                .find_region_mut(addr)
                .unwrap_or_else(|| panic!("no region for {addr:#010X}"));
            assert_eq!(region.name(), "sram", "unexpected owner of {addr:#010X}");
            let mut cycles = 0;
            bus.write_u32(addr, 0xA5A5_A5A5, &mut cycles)
                .unwrap_or_else(|e| panic!("write {addr:#010X}: {e:?}"));
            assert_eq!(bus.read_u32(addr, &mut cycles).expect("read"), 0xA5A5_A5A5);
        }
    }

    #[test]
    fn image_summary_keeps_the_run_format() {
        let elf = infinite_loop_elf();
        let machine = Machine::new(&elf, &MachineOptions::default()).expect("machine");
        assert!(
            machine
                .image_description()
                .starts_with("elf: entry=0x10000001")
        );
        assert_eq!(machine.boot_vectors().map(|(_, pc)| pc), Some(0x1000_0000));
    }

    /// Build one 512-byte UF2 block with a family ID / 构造一个带 family ID 的 UF2 块
    fn uf2_block(family: u32, address: u32, payload: &[u8]) -> Vec<u8> {
        let mut block = vec![0u8; 512];
        block[0..4].copy_from_slice(&0x0A32_4655u32.to_le_bytes());
        block[4..8].copy_from_slice(&0x9E5D_5157u32.to_le_bytes());
        // Family-ID-present flag. / 置位 family ID 存在标志。
        block[8..12].copy_from_slice(&0x0000_2000u32.to_le_bytes());
        block[12..16].copy_from_slice(&address.to_le_bytes());
        block[16..20].copy_from_slice(&u32::try_from(payload.len()).unwrap().to_le_bytes());
        block[20..24].copy_from_slice(&0u32.to_le_bytes());
        block[24..28].copy_from_slice(&1u32.to_le_bytes());
        block[28..32].copy_from_slice(&family.to_le_bytes());
        block[32..32 + payload.len()].copy_from_slice(payload);
        block[508..512].copy_from_slice(&0x0AB1_6F30u32.to_le_bytes());
        block
    }

    #[test]
    fn uf2_family_selects_the_cpu_architecture() {
        // RISC-V family must auto-select Hazard3; the explicit override wins.
        // RISC-V family 必须自动选择 Hazard3；显式覆盖优先。
        let riscv = uf2_block(0xE48B_FF5A, FLASH_XIP_BASE, &[0u8; 4]);
        let mut bus = SimpleBus::new();
        bus.register_region(Box::new(RamRegion::new("flash", FLASH_XIP_BASE, 0x1000)))
            .expect("flash region");
        let (arch, entry, description) = load_image(&riscv, &mut bus, None).expect("load");
        assert_eq!(arch, ArchArg::RiscV);
        assert_eq!(entry, None);
        assert!(description.contains("RISC-V"));
        let (arch, _, _) = load_image(&riscv, &mut bus, Some(ArchArg::Arm)).expect("load override");
        assert_eq!(arch, ArchArg::Arm);

        let arm = uf2_block(0xE48B_FF59, FLASH_XIP_BASE, &[0u8; 4]);
        let (arch, _, _) = load_image(&arm, &mut bus, None).expect("load arm");
        assert_eq!(arch, ArchArg::Arm);
    }

    // ---- core 1: launch protocol, stepping, IRQ routing ---------------------

    /// SIO `FIFO_ST` as seen from core 0 / core0 视角的 SIO `FIFO_ST`
    const SIO_FIFO_ST: u32 = 0xD000_0050;

    /// SIO `FIFO_WR` / SIO `FIFO_WR`
    const SIO_FIFO_WR: u32 = 0xD000_0054;

    /// SIO `FIFO_RD` / SIO `FIFO_RD`
    const SIO_FIFO_RD: u32 = 0xD000_0058;

    /// `PSM_FRCE_OFF` SET alias / `PSM_FRCE_OFF` 的 SET 别名
    const PSM_FRCE_OFF_SET: u32 = 0x4001_A004;

    /// `PSM_FRCE_OFF` CLR alias / `PSM_FRCE_OFF` 的 CLR 别名
    const PSM_FRCE_OFF_CLR: u32 = 0x4001_B004;

    /// `PSM_FRCE_OFF.PROC1` / `PSM_FRCE_OFF.PROC1`
    const PSM_PROC1: u32 = 0x0100_0000;

    /// Core-1 code region inside SRAM / SRAM 中的 core1 代码区
    const CORE1_CODE: u32 = SRAM_BASE + 0x8000;

    /// Core-1 vector table inside SRAM / SRAM 中的 core1 向量表
    const CORE1_VTOR: u32 = SRAM_BASE + 0x1000;

    /// Core-1 initial stack pointer / core1 初始栈指针
    const CORE1_STACK: u32 = SRAM_BASE + 0xA000;

    /// Shared sentinel word / 共享哨兵字
    const SENTINEL: u32 = SRAM_BASE + 0x9000;

    /// `LDR r0,[pc,#4]; LDR r1,[pc,#8]; STR r0,[r1]; B .`
    const STORE_PROGRAM: [u16; 4] = [0x4801, 0x4902, 0x6008, 0xE7FE];

    /// `LDR r1,[pc,#8]; LDR r0,[r1]; STR r0,[r1,#4]; B .`
    const COPY_PROGRAM: [u16; 4] = [0x4902, 0x6808, 0x6048, 0xE7FE];

    /// `LDR r1,[pc,#8]; LDR r0,[r1]; ADDS r0,#1; STR r0,[r1]; B`
    const COUNTER_LOOP: [u16; 5] = [0x4902, 0x6808, 0x3001, 0x6008, 0xE7FB];

    /// Thumb `b .` / Thumb `b .`
    const SPIN_PROGRAM: [u16; 1] = [0xE7FE];

    /// Write a halfword program / 写入半字程序
    fn bind_halfwords(machine: &mut Machine, base: u32, halfwords: &[u16]) {
        let mut cycles = 0u64;
        for (index, halfword) in halfwords.iter().enumerate() {
            machine
                .bus
                .write_u16(base + index as u32 * 2, *halfword, &mut cycles)
                .expect("code write");
        }
    }

    /// Write `value` to `target` from the store program / 用存储程序把值写入目标
    fn bind_store_program(machine: &mut Machine, base: u32, target: u32, value: u32) {
        bind_halfwords(machine, base, &STORE_PROGRAM);
        write_word(machine, base + 0x08, value);
        write_word(machine, base + 0x0C, target);
    }

    /// Write a self-contained increment loop over `counter` / 写入自包含自增循环
    fn bind_counter_program(machine: &mut Machine, base: u32, counter: u32) {
        bind_halfwords(machine, base, &COUNTER_LOOP);
        write_word(machine, base + 0x0C, counter);
    }

    /// Push the SDK's `{0,0,1,vtor,sp,entry}` stream and check every echo
    /// 写入 SDK 的 `{0,0,1,vtor,sp,entry}` 命令流并校验每个回显
    fn push_launch(machine: &mut Machine, vtor: u32, sp: u32, entry: u32) {
        for word in [0, 0, 1, vtor, sp, entry] {
            write_word(machine, SIO_FIFO_WR, word);
            machine.peripherals.service_core1_peer();
            assert_eq!(
                read_word(machine, SIO_FIFO_RD),
                word,
                "the boot-ROM peer must echo {word:#010X}"
            );
        }
        assert!(machine.peripherals.core1_launch_pending());
    }

    #[test]
    fn the_launch_stream_boots_core1_from_entry_sp_and_vtor() {
        // 命令流被识别后 core1 从 entry 启动，SP 与 VTOR 取自同一命令流。
        // Once the stream is recognized core 1 boots at entry, with SP and VTOR
        // taken from the same command stream.
        let mut machine = machine_with_budget(1_000);
        bind_halfwords(&mut machine, CORE1_CODE, &SPIN_PROGRAM);
        assert_eq!(machine.core1_state(), Some(CpuState::Reset));

        push_launch(&mut machine, CORE1_VTOR, CORE1_STACK, CORE1_CODE | 1);
        assert_eq!(machine.core1_state(), Some(CpuState::Reset));
        machine.step().expect("launch step");

        assert_eq!(machine.core1_state(), Some(CpuState::Running));
        assert_eq!(machine.core1_pc(), Some(CORE1_CODE));
        assert_eq!(machine.core1_sp(), Some(CORE1_STACK));
        assert_eq!(
            machine.core1_system_register(0xE000_ED08),
            Some(CORE1_VTOR),
            "core1 VTOR comes from the launch stream / core1 VTOR 来自启动流"
        );
        assert_eq!(machine.core1_launch_count(), 1);
        assert_eq!(machine.core1_halt_reason(), None);
    }

    #[test]
    fn core1_never_touches_shared_sram_before_release() {
        // 未释放的 core1 处于 Reset：共享哨兵必须保持原值。
        // An unreleased core 1 stays in Reset, so the shared sentinel keeps its
        // original value.
        let mut machine = machine_with_budget(1_000);
        bind_store_program(&mut machine, CORE1_CODE, SENTINEL, 0x1234_5678);
        write_word(&mut machine, SENTINEL, 0xDEAD_BEEF);

        machine.run_cycles(200);

        assert_eq!(machine.core1_state(), Some(CpuState::Reset));
        assert_eq!(machine.core1_pc(), Some(0));
        assert_eq!(read_word(&mut machine, SENTINEL), 0xDEAD_BEEF);
        assert_eq!(machine.core1_launch_count(), 0);
    }

    #[test]
    fn core1_writes_shared_sram_and_core0_reads_it() {
        // core1 运行手写 Thumb 程序写共享 SRAM，core0 随后用同一总线读出。
        // Core 1 runs a hand-written Thumb program that fills shared SRAM, then
        // core 0 reads it back through the same bus.
        let mut machine = machine_with_budget(1_000);
        bind_store_program(&mut machine, CORE1_CODE, SENTINEL, 0x5EED_1234);
        push_launch(&mut machine, CORE1_VTOR, CORE1_STACK, CORE1_CODE | 1);

        machine.run_cycles(4);
        assert_eq!(machine.core1_state(), Some(CpuState::Running));
        assert_eq!(read_word(&mut machine, SENTINEL), 0x5EED_1234);

        bind_halfwords(&mut machine, SRAM_BASE + 0x8400, &COPY_PROGRAM);
        write_word(&mut machine, SRAM_BASE + 0x840C, SENTINEL);
        machine.write_register(15, SRAM_BASE + 0x8400);
        for _ in 0..3 {
            machine.step().expect("core0 copy");
        }
        assert_eq!(
            read_word(&mut machine, SENTINEL + 4),
            0x5EED_1234,
            "core0 copied core1's store / core0 复制了 core1 的写入"
        );
    }

    #[test]
    fn core0_and_core1_keep_independent_register_files() {
        // 两核寄存器文件独立：core1 的栈与寄存器改变不影响 core0。
        // The register files are independent: core 1's stack and registers do
        // not disturb core 0's.
        let mut machine = machine_with_budget(1_000);
        bind_halfwords(&mut machine, CORE1_CODE, &SPIN_PROGRAM);
        machine.write_register(0, 0xAAAA_AAAA);
        let core0_sp = machine.sp();

        push_launch(&mut machine, CORE1_VTOR, CORE1_STACK, CORE1_CODE | 1);
        machine.step().expect("launch step");

        assert_eq!(machine.core1_register(0), Some(0));
        assert_eq!(machine.core1_sp(), Some(CORE1_STACK));
        assert_eq!(machine.register(0), 0xAAAA_AAAA);
        assert_eq!(machine.sp(), core0_sp);
    }

    #[test]
    fn both_cores_advance_their_own_counters() {
        // 双核交替步进：每次 step 两个核心各执行一条指令，计数器同步增长。
        // Alternating steps: each `step` retires one instruction per core, so the
        // two counters grow together.
        let mut machine = machine_with_budget(1_000);
        bind_counter_program(&mut machine, CORE1_CODE, SENTINEL);
        bind_counter_program(&mut machine, SRAM_BASE + 0x8400, SENTINEL + 4);
        push_launch(&mut machine, CORE1_VTOR, CORE1_STACK, CORE1_CODE | 1);
        machine.write_register(15, SRAM_BASE + 0x8400);

        machine.run_cycles(40);

        assert_eq!(read_word(&mut machine, SENTINEL), 10);
        assert_eq!(read_word(&mut machine, SENTINEL + 4), 10);
    }

    #[test]
    fn a_halted_core1_does_not_stop_core0_or_consume_budget() {
        // core1 的总线错误只停机 core1：core0 继续跑，预算按 core0 步数计。
        // A core-1 bus error only stops core 1: core 0 keeps running and the
        // budget still counts core-0 steps.
        let mut machine = machine_with_budget(20);
        push_launch(&mut machine, CORE1_VTOR, CORE1_STACK, 0x3000_0000 | 1);

        machine.run_cycles(5);
        assert_eq!(machine.core1_state(), Some(CpuState::Halted));
        assert!(machine.core1_halt_reason().is_some());
        assert_eq!(machine.halt_reason(), None, "core0 must survive");
        assert_eq!(machine.steps(), 5);

        machine.run_cycles(5);
        assert_eq!(machine.steps(), 10);

        machine.run_cycles(100);
        assert_eq!(machine.steps(), 20);
        assert!(machine.budget_exhausted());
    }

    #[test]
    fn core1_pushes_a_fifo_frame_that_core0_pops() {
        // core1 写自己的 FIFO_WR 后，core0 的 FIFO_RD / SIO_IRQ_FIFO 必须看到它。
        // After core 1 writes its own FIFO_WR, core 0's FIFO_RD and SIO_IRQ_FIFO
        // must observe the frame.
        let mut machine = machine_with_budget(1_000);
        bind_store_program(&mut machine, CORE1_CODE, SIO_FIFO_WR, 0);
        push_launch(&mut machine, CORE1_VTOR, CORE1_STACK, CORE1_CODE | 1);
        let fifo_line = 1u64 << rp2350_periph::map::SIO_IRQ_FIFO;

        machine.run_cycles(4);

        assert_ne!(
            read_word(&mut machine, SIO_FIFO_ST) & 1,
            0,
            "core0's receive FIFO has the frame / core0 收件箱有数据"
        );
        assert_ne!(machine.peripherals.irq_mask() & fifo_line, 0);
        assert_eq!(read_word(&mut machine, SIO_FIFO_RD), 0);
        assert_eq!(
            machine.peripherals.irq_mask() & fifo_line,
            0,
            "popping releases the line / 弹出后释放中断线"
        );
    }

    #[test]
    fn core1_takes_a_dma_completion_interrupt() {
        // DMA_IRQ_0 属于共享线；core1 使能后必须进入自己的 hstx 处理程序。
        // DMA_IRQ_0 is a shared line; once core 1 enables it, its own handler
        // must run.
        let mut machine = machine_with_budget(1_000);
        let handler = SRAM_BASE + 0x8800;
        let irq = rp2350_periph::map::DMA_IRQ_0;
        bind_halfwords(&mut machine, handler, &SPIN_PROGRAM);
        bind_halfwords(&mut machine, CORE1_CODE, &SPIN_PROGRAM);
        write_word(&mut machine, CORE1_VTOR, CORE1_STACK);
        write_word(&mut machine, CORE1_VTOR + 4, CORE1_CODE | 1);
        write_word(
            &mut machine,
            CORE1_VTOR + (16 + u32::from(irq)) * 4,
            handler | 1,
        );
        push_launch(&mut machine, CORE1_VTOR, CORE1_STACK, CORE1_CODE | 1);
        machine.step().expect("launch step");
        machine
            .core1
            .as_mut()
            .expect("core1")
            .system_write(0xE000_E100, 1u32 << u32::from(irq));

        program_channel(
            &mut machine,
            0,
            SRAM_BASE + 0x1000,
            SRAM_BASE + 0x2000,
            1,
            0,
        );
        write_word(&mut machine, DMA_BASE + 0x404, 1);
        machine.run_cycles(2 * PUMP_INTERVAL);

        assert_eq!(
            machine.core1_pc(),
            Some(handler),
            "core1 entered its DMA_IRQ_0 handler / core1 已进入 DMA_IRQ_0 处理程序"
        );
    }

    #[test]
    fn psm_reset_then_relaunch_restarts_core1() {
        // 置位 PROC1 硬复位 core1 并清空启动状态；清除后重新走一遍启动协议。
        // Setting PROC1 hard-resets core 1 and drops the launch state; clearing
        // it lets the launch protocol run again.
        let mut machine = machine_with_budget(1_000);
        bind_halfwords(&mut machine, CORE1_CODE, &SPIN_PROGRAM);
        push_launch(&mut machine, CORE1_VTOR, CORE1_STACK, CORE1_CODE | 1);
        machine.step().expect("launch step");
        assert_eq!(machine.core1_state(), Some(CpuState::Running));
        assert_eq!(machine.core1_launch_count(), 1);

        write_word(&mut machine, PSM_FRCE_OFF_SET, PSM_PROC1);
        machine.step().expect("reset step");
        assert_eq!(machine.core1_state(), Some(CpuState::Reset));
        assert_eq!(machine.core1_pc(), Some(0));

        write_word(&mut machine, PSM_FRCE_OFF_CLR, PSM_PROC1);
        machine.peripherals.service_core1_peer();
        assert_eq!(
            read_word(&mut machine, SIO_FIFO_RD),
            0,
            "boot ROM acknowledges the release / boot ROM 确认释放"
        );

        push_launch(&mut machine, CORE1_VTOR, CORE1_STACK, CORE1_CODE | 1);
        machine.step().expect("relaunch step");
        assert_eq!(machine.core1_state(), Some(CpuState::Running));
        assert_eq!(machine.core1_launch_count(), 2);
    }

    #[test]
    fn gdb_single_steps_advance_core1() {
        // GDB `resume` 走 `Machine::step`，因此单步必须同时推进 core1。
        // GDB `resume` goes through `Machine::step`, so a single step must also
        // advance core 1.
        let mut machine = machine_with_budget(1_000);
        bind_counter_program(&mut machine, CORE1_CODE, SENTINEL);
        push_launch(&mut machine, CORE1_VTOR, CORE1_STACK, CORE1_CODE | 1);
        machine.step().expect("launch step");
        let first_pc = machine.core1_pc().expect("core1 pc");
        assert_eq!(read_word(&mut machine, SENTINEL), 0);

        machine.step().expect("single step");
        assert_ne!(machine.core1_pc(), Some(first_pc));
        machine.step().expect("single step");
        machine.step().expect("single step");
        assert_eq!(read_word(&mut machine, SENTINEL), 1);
    }

    /// `emerald.uf2` used by the ignore-gated smoke test / 冒烟测试使用的固件
    const EMERALD_UF2: &str =
        r"E:\Source\mcu\pico\rp2350\gba\pokeemerald-rp2350\rp2350\hw\build_emerald\emerald.uf2";

    #[test]
    #[ignore = "long firmware smoke test / 长固件冒烟测试"]
    fn emerald_runs_core1_hstx_without_bus_errors() {
        // 真实固件冒烟：`main` 用 `multicore_launch_core1_with_stack` 启动
        // core1_render，其第一件事是 hstx_display_start，因此 HSTX 输出字数必须
        // 变为非零，且不得出现总线错误。
        // Real-firmware smoke test: `main` launches `core1_render` through
        // `multicore_launch_core1_with_stack`; its first action is
        // `hstx_display_start`, so HSTX output words must become non-zero and no
        // bus error may occur.
        let Ok(bytes) = std::fs::read(EMERALD_UF2) else {
            return;
        };
        let options = MachineOptions {
            arch: None,
            max_cycles: 500_000_000,
            strict_decode: false,
        };
        let mut machine = Machine::new(&bytes, &options).expect("machine");
        while machine.halt_reason().is_none() && !machine.budget_exhausted() {
            machine.step().expect("step");
        }
        machine.finalize();

        assert_eq!(machine.halt_reason(), None);
        assert_eq!(machine.core1_state(), Some(CpuState::Running));
        assert!(machine.core1_launch_count() >= 1);
        assert!(
            machine.hstx_words() > 0,
            "hstx words={} dma={} / HSTX 输出字数为 0",
            machine.hstx_words(),
            machine.dma_transfers()
        );
        // Pixel-clock pacing keeps the expander in step with the DMA ring, so a
        // healthy scanout shows no starvation episodes and (at most) a single
        // start-up command/data hiccup.
        // 像素时钟节拍使扩展器与 DMA 环同步，因此健康扫描不应有断流，命令/数据
        // 相位错位最多出现一次（启动期）。
        let hstx = machine.peripherals.hstx();
        assert_eq!(
            hstx.output_underrun(),
            0,
            "hstx underruns must stay at zero / HSTX 不应断流"
        );
        assert!(
            hstx.unknown_commands() <= 1,
            "unknown commands: {} / 未知命令过多",
            hstx.unknown_commands()
        );
        assert!(machine.dma_transfers() > 0);
    }
}
