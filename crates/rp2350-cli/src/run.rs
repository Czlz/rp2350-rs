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

/// Ring buffer of the last executed `(step, PC)` pairs
/// 最近执行过的 `(步数, PC)` 定长环形缓冲
///
/// `[tail]` 与故障回溯只需要最后 [`TAIL_TRACE_STEPS`] 条记录。定长数组把每步的
/// 记录工作降为两次存储加一次索引递增，去掉了 `VecDeque` 的容量分支与取模。
/// The `[tail]` and fault traces only need the last [`TAIL_TRACE_STEPS`] pairs.
/// A fixed array turns the per-step record into two stores plus an index bump and
/// removes the `VecDeque` capacity branch and modulo.
struct RecentSteps {
    /// Stored pairs, oldest overwritten first / 存储的条目，最旧的先被覆盖
    items: [(u64, u32); TAIL_TRACE_STEPS],
    /// Next write cursor / 下一个写入位置
    next: usize,
    /// Number of valid entries (saturates at the capacity) / 有效条目数（达到容量即饱和）
    len: usize,
}

impl RecentSteps {
    /// Create an empty ring / 创建空环形缓冲
    const fn new() -> Self {
        Self {
            items: [(0, 0); TAIL_TRACE_STEPS],
            next: 0,
            len: 0,
        }
    }

    /// Record one executed step, dropping the oldest when full
    /// 记录一步；满时覆盖最旧的一条
    #[inline]
    fn push(&mut self, step: u64, pc: u32) {
        self.items[self.next] = (step, pc);
        self.next = (self.next + 1) & (TAIL_TRACE_STEPS - 1);
        if self.len < TAIL_TRACE_STEPS {
            self.len += 1;
        }
    }

    /// Iterate the recorded pairs from oldest to newest
    /// 从最旧到最新遍历已记录条目
    fn iter(&self) -> impl DoubleEndedIterator<Item = (u64, u32)> + '_ {
        let start = (self.next + TAIL_TRACE_STEPS - self.len) & (TAIL_TRACE_STEPS - 1);
        (0..self.len).map(move |i| self.items[(start + i) & (TAIL_TRACE_STEPS - 1)])
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
    recent: RecentSteps,
    /// PPB write epoch already replayed into the core / 已回放到核的 PPB 写世代
    ppb_epoch_seen: u64,
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
                // 本机器在 XIP 窗口上映射的是只读 `FlashRegion`，因此取指缓存是精确的。
                // This machine maps a read-only `FlashRegion` over the XIP windows,
                // so the fetch cache is exact here.
                arm.set_flash_fetch_cache(true);
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
                core1.set_flash_fetch_cache(true);
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
            recent: RecentSteps::new(),
            ppb_epoch_seen: 0,
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

    /// Boot peripheral handles / 启动外设句柄
    ///
    /// Read-only view of the peripherals mounted by [`Self::new`], so a host can
    /// inspect pads, PIO blocks, DMA and HSTX without reaching into the bus.
    /// [`Self::new`] 挂载的外设的只读视图，使宿主无需操作总线即可检查 pad、
    /// PIO 块、DMA 与 HSTX。
    #[must_use]
    pub fn peripherals(&self) -> &BootPeripherals {
        &self.peripherals
    }

    /// Mutable boot peripheral handles / 可变启动外设句柄
    ///
    /// Needed by hosts that install a capture tap through
    /// [`BootPeripherals::set_audio_tap`]; everything else uses the shared
    /// handles from [`Self::peripherals`].
    /// 供通过 [`BootPeripherals::set_audio_tap`] 安装捕获 tap 的宿主使用；其余场景
    /// 使用 [`Self::peripherals`] 返回的共享句柄。
    pub fn peripherals_mut(&mut self) -> &mut BootPeripherals {
        &mut self.peripherals
    }

    /// Drive the external level of one pad / 驱动一个 pad 的外部电平
    ///
    /// Unlike [`Self::set_pad_levels`] this only claims the requested pin, so an
    /// input injected on one pin never disturbs another.
    /// 与 [`Self::set_pad_levels`] 不同，这里只声明目标引脚，因此为单个引脚注入
    /// 的输入不会影响其它引脚。
    pub fn set_pad_pin(&self, pin: u32, high: bool) {
        self.peripherals.pads().set_external_pin(pin, high);
    }

    /// Drive pads `0..=63` from one level mask / 按电平掩码驱动 `0..=63` 号 pad
    pub fn set_pad_levels(&self, levels: u64) {
        self.peripherals.pads().set_external(levels);
    }

    /// Read up to `len` bytes starting at `addr` / 从 `addr` 起读取最多 `len` 字节
    ///
    /// Unlike [`Self::dump_memory`] this is a flat byte copy meant for frame
    /// buffers: it stops at the first unmapped address instead of substituting a
    /// marker word, so the caller sees the readable prefix.
    /// `read_block(addr, len).len() == len` therefore proves the whole block was
    /// readable.
    /// 与 [`Self::dump_memory`] 不同，这是面向帧缓冲的连续字节拷贝：遇到首个未映射
    /// 地址即停止，而不是填充标记字，因此调用方看到的是可读前缀。
    /// `read_block(addr, len).len() == len` 即可证明整块可读。
    pub fn read_block(&mut self, addr: u32, len: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(len);
        let mut cycles = 0u64;
        for offset in 0..len {
            let address = addr.wrapping_add(offset as u32);
            match self.bus.read_u8(address, &mut cycles) {
                Ok(byte) => out.push(byte),
                Err(_) => break,
            }
        }
        out
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

    /// Whether any trace line or spin warning is queued
    /// 是否有排队的轨迹行或自旋告警
    ///
    /// The `run` loop checks this before draining so an idle step does not move
    /// the notice buffer at all.
    /// `run` 循环在取走前先检查它，使空闲步完全不动通知缓冲。
    #[must_use]
    pub fn notices_pending(&self) -> bool {
        !self.notices.is_empty()
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
        let recent: Vec<(u64, u32)> = self.recent.iter().collect();
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
        self.recent.push(self.steps, pc);
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
    /// Thin wrapper over [`Self::step_cores`]`(true, true)`, so the dual-core
    /// semantics of `run`, `repl` and `perfbench` are unchanged.
    /// 这是 [`Self::step_cores`]`(true, true)` 的薄封装，因此 `run`、`repl` 与
    /// `perfbench` 的双核语义保持不变。
    ///
    /// # Errors
    /// Propagates bus errors raised while dispatching a core-0 boot ROM trap.
    /// Core-1 faults are captured in [`Self::core1_halt_reason`] and never abort
    /// core 0.
    /// 传播 core0 boot ROM 陷阱分发期间的总线错误。core1 故障记录在
    /// [`Self::core1_halt_reason`] 中，绝不影响 core0。
    pub fn step(&mut self) -> Result<CpuState> {
        self.step_cores(true, true)
    }

    /// Execute one instruction on the selected cores only
    /// 仅在选中的核心上各执行一条指令
    ///
    /// `core0`/`core1` choose which cores retire one instruction. An unselected
    /// core keeps its state and PC untouched, and a selected core that is not
    /// [`CpuState::Running`] is not advanced either. The step still performs the
    /// shared peripheral work of [`Self::step`] exactly once: the machine
    /// timeline (step counter, budget, timebase) advances, the PIO and DMA
    /// batches are pumped, PPB system writes are replayed, and SEV/peer
    /// handshakes plus IRQ lines are forwarded. [`Self::run_cycles`] always
    /// advances both cores; per-core stepping is meant for debugger
    /// single-stepping.
    /// `core0`/`core1` 选择哪些核心各退役一条指令。未选中的核心保持状态与 PC
    /// 不变，被选中但不在 [`CpuState::Running`] 的核心同样不推进。该步仍恰好执行
    /// 一次 [`Self::step`] 的共享外设工作：机器时间线（步数、预算、时基）推进，
    /// PIO 与 DMA 批次泵送，PPB 系统写入回放，SEV/对端握手与中断线转发。
    /// [`Self::run_cycles`] 始终推进双核；按核步进面向调试器单步。
    ///
    /// # Errors
    /// Same conditions as [`Self::step`]. / 错误条件与 [`Self::step`] 相同。
    pub fn step_cores(&mut self, core0: bool, core1: bool) -> Result<CpuState> {
        self.step_cores_with_trace(core0, core1, None)
    }

    /// Execute one instruction with a `run`-style trace line
    /// 执行一条指令并生成 `run` 风格的轨迹行
    ///
    /// Traces always describe core 0, so this keeps advancing both cores.
    /// 轨迹行始终描述 core0，因此仍然推进双核。
    ///
    /// # Errors
    /// Same conditions as [`Self::step`]. / 错误条件与 [`Self::step`] 相同。
    pub fn trace_step(&mut self, index: usize) -> Result<CpuState> {
        self.step_cores_with_trace(true, true, Some(index))
    }

    /// Advance the guest by at most `cycles` instructions
    /// 最多推进 `cycles` 条指令
    ///
    /// This is the callback used by [`rp2350_usb_host::CdcLink`]: it runs the
    /// same boot-ROM dispatch, interrupt pump, timebase advance and UART
    /// collection as the `run` path, and stops at the machine budget. Every
    /// iteration retires one instruction on **both** cores (plain
    /// [`Self::step`]); per-core progress is only available through
    /// [`Self::step_cores`].
    /// 这是 [`rp2350_usb_host::CdcLink`] 使用的回调：执行与 `run` 路径相同的
    /// boot ROM 分发、中断泵送、时基推进与 UART 收集，并在机器预算处停止。每次
    /// 迭代在**两个**核心上各退役一条指令（普通 [`Self::step`]）；按核推进只能
    /// 通过 [`Self::step_cores`] 获得。
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

    /// Step the selected cores and run the shared peripheral path once
    /// 推进选中的核心并执行一次共享外设路径
    ///
    /// Core 0's instruction path already carries the common peripheral work
    /// (DMA/PIO batches, PPB replay, SEV/IRQ forwarding, timebase), so a
    /// core-1-only step runs the same pieces itself instead of duplicating a
    /// second step loop.
    /// core0 的指令路径本来就携带公共外设工作（DMA/PIO 批次、PPB 回放、
    /// SEV/中断转发、时基），因此仅 core1 的步进直接复用这些片段，而不是另建一套
    /// 步进循环。
    fn step_cores_with_trace(
        &mut self,
        core0: bool,
        core1: bool,
        trace: Option<usize>,
    ) -> Result<CpuState> {
        if self.halt.is_some() || self.budget_exhausted() {
            return Ok(CpuState::Halted);
        }
        let state = if core0 {
            self.execute_core0_step(trace)?
        } else {
            CpuState::Running
        };
        // core1 只在被选中时推进；未选中时状态与 PC 保持原样。
        // Core 1 advances only when selected; when unselected its state and PC
        // stay untouched.
        let core1_advanced = core1 && self.step_core1();
        if !core0 && core1_advanced {
            // core0 未选中时公共外设路径仍需每步一次：步数/预算、DMA 与 PIO
            // 批量泵送、PPB 回放、SEV 转发与时基推进都与双核 `step` 一致。
            // With core 0 unselected the shared peripheral path still runs once
            // per step: timeline/budget, DMA and PIO batches, PPB replay, SEV
            // forwarding and the timebase match the dual-core `step`.
            self.steps += 1;
            self.pump_dma_batch();
            self.pump_pio_batch();
            // 仅 core1 的步进不产生 core0 轨迹行，因此不传 trace。
            // A core-1-only step produces no core-0 trace line, so `trace` is
            // not forwarded.
            self.after_running_step(None);
        }
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
    /// Returns whether an instruction was actually retired, so the caller knows
    /// whether the machine timeline advanced.
    /// 在执行 core1 之前处理两种会改变其状态的握手（`PSM` 硬复位与已完成的 SIO
    /// FIFO 启动）。指令执行前后把 SIO 视图切到 core1，使 `FIFO_WR`、`FIFO_RD`、
    /// 门铃与 `CPUID` 遵循 core1 的方向；PPB 系统写入回放到 core1，使其
    /// `VTOR`、`CPACR` 与 NVIC 与 core0 相互独立。core1 故障只停止 core1。
    /// 返回是否真的退役了一条指令，供调用方判断机器时间线是否推进。
    fn step_core1(&mut self) -> bool {
        if self.halt.is_some() || self.budget_exhausted() {
            return false;
        }
        // PSM 置位 FRCE_OFF.PROC1：硬复位 core1 并重新启用 boot ROM 回显桩。
        // FRCE_OFF.PROC1 set: hard-reset core 1 and re-arm the boot-ROM echo stub.
        if self.peripherals.core1_reset_pending() && self.peripherals.take_core1_reset() {
            self.peripherals.reset_core1_boot();
            self.core1_halt = None;
            if self.core1.is_some() {
                let mut core1 = CortexM33Core::new(CpuId::Core1);
                core1.set_strict_decode(self.strict_decode);
                core1.set_flash_fetch_cache(true);
                self.core1 = Some(core1);
            }
        }
        if self.peripherals.core1_launch_pending() {
            if let Some(launch) = self.peripherals.take_core1_launch() {
                self.boot_core1(launch);
            }
        }
        let Some(core1) = self.core1.as_mut() else {
            return false;
        };
        if core1.state() != CpuState::Running {
            return false;
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
        true
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
        // 合成 boot ROM 的全部陷阱桩都位于 ROM 窗口内，窗口外的 PC 无需线性扫描
        // 桩表；这一步把每步的桩查询降为一次范围比较。
        // Every synthetic boot ROM trap stub lives inside the ROM window, so a PC
        // outside it needs no stub-table scan; this reduces the per-step stub
        // lookup to one range comparison.
        let in_rom_window = pc.wrapping_sub(rp2350_bootrom::ROM_BASE)
            < u32::try_from(rp2350_bootrom::ROM_SIZE).unwrap_or(u32::MAX);
        if self.bootrom_applicable && in_rom_window && self.bootrom.handles(pc) {
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
                    let pc = self.core.pc();
                    self.after_running_step(Some(pc));
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

    /// Post-step housekeeping for core 0 and the shared peripherals
    /// core0 与共享外设的步后维护
    ///
    /// `core0_pc` is `Some` only when core 0 actually retired an instruction;
    /// the spin detector then watches that PC window. A core-1-only step passes
    /// `None` so core 0's frozen PC never looks like a polling spin. The PPB
    /// replay, IRQ/SEV forwarding and timebase work always run, because they are
    /// the shared path every step uses.
    /// 仅当 core0 真的退役了一条指令时 `core0_pc` 才是 `Some`，自旋检测随后观察
    /// 该 PC 窗口。仅 core1 的步进传 `None`，避免 core0 冻结的 PC 被误判为轮询
    /// 自旋。PPB 回放、中断/SEV 转发与时基工作始终执行，因为它们是每步共享路径。
    fn after_running_step(&mut self, core0_pc: Option<u32>) {
        // 紧凑自旋通常意味着固件在轮询一个永远不变的 MMIO 值（未建模设备静默
        // 读 0）或执行延时循环，会无声耗尽指令预算。PC 连续停在同一小窗口内
        // 达到阈值就告警一次，并给出窗口范围以便反汇编定位。
        // A tight spin usually means the firmware polls a register that never
        // changes (unmodelled devices read zero) or runs a delay loop, silently
        // burning the budget. Warn once when the PC stays inside one small window
        // and print the window so the spin point can be disassembled.
        if let Some(pc) = core0_pc {
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
        }
        // 系统控制寄存器写入必须在下一条指令前生效：固件写 CPACR 后紧跟
        // `mrc p4`/VFP 指令，按 64 步批量回放会让核误判协处理器未使能并触发
        // NOCP UsageFault。中断线保持节流，避免电平中断被每一步重复投递。
        // PPB 写世代未推进时回放队列必然为空，直接跳过（省去一次互斥锁）。
        // System-control writes must land before the next instruction: firmware
        // writes CPACR immediately before `mrc p4`/VFP instructions, so a
        // 64-step batch replay makes the core raise a spurious NOCP UsageFault.
        // IRQ lines stay throttled to avoid re-asserting a level IRQ each step.
        // An unchanged PPB write epoch proves the replay queue is empty, so the
        // drain (and its mutex) is skipped entirely.
        let epoch = self.bus.ppb_write_epoch();
        if epoch != self.ppb_epoch_seen {
            self.ppb_epoch_seen = epoch;
            replay_system_writes(&mut self.core, &self.peripherals);
        }
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
        if machine.notices_pending() {
            drain_notices(&mut machine, &mut stdout)?;
        }
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
/// `TAIL_TRACE_STEPS` instructions makes the offending branch locatable, the
/// register snapshot shows how its operands were produced, and the memory dump
/// of `R4`/`SP` reveals corrupted structures.
/// `repl` 与 `perfbench` 不输出逐步轨迹，客户机总线故障只剩一个最终 PC；
/// 打印停机前 `TAIL_TRACE_STEPS` 条指令即可定位罪魁分支，寄存器快照说明
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
    // 只遍历已断言的位置：`trailing_zeros` 逐位清除，顺序与 0..64 扫描一致。
    // Walk asserted bits only; ascending order matches the 0..64 scan.
    let mut mask = peripherals.irq_mask();
    while mask != 0 {
        let irq = mask.trailing_zeros() as u16;
        mask &= mask - 1;
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
    let mut mask = peripherals.core1_irq_mask();
    while mask != 0 {
        let irq = mask.trailing_zeros() as u16;
        mask &= mask - 1;
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
mod tests;
