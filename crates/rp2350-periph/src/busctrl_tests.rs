//! BUSCTRL tests: priority hints, performance counters, fabric accounting
//! BUSCTRL 测试：优先级提示、性能计数器与总线矩阵记账
//!
//! The counter tests drive [`BusPerf`] directly where the exact event stream
//! matters and through a real DMA + bus fixture where the register path matters.
//! 需要精确事件流的测试直接驱动 [`BusPerf`]，寄存器通路的测试则使用真实的
//! DMA 与总线夹具。

use rp2350_core::bus::AddressBus;
use rp2350_core::memory::{RamRegion, SimpleBus};
use rp2350_core::mmio::{MmioDevice, mount_device};

use crate::boot::BootPeripherals;
use crate::busctrl::{
    BUS_PRIORITY, BUS_PRIORITY_ACK, BUS_PRIORITY_MASK, BusCtrl, BusPerf, BusPort,
    DMA_R_PRIORITY_MASK, DMA_W_PRIORITY_MASK, PERF_EVENT_ACCESS, PERF_EVENT_ACCESS_CONTESTED,
    PERF_EVENT_STALL_DOWNSTREAM, PERF_EVENT_STALL_UPSTREAM, PERFCTR_EN, PERFCTR_MASK, PERFCTR0,
    PERFCTR1, PERFSEL_MASK, PERFSEL_RESET, PERFSEL0, PERFSEL1, PORT_COUNT, port_of_address,
};
use crate::dma::{
    CTRL_DATA_SIZE_SHIFT, CTRL_EN, CTRL_INCR_READ, CTRL_INCR_WRITE, CTRL_TREQ_SEL_SHIFT, Dma,
    TREQ_PERMANENT,
};
use crate::irq::IrqAssert;
use crate::map;

/// Base address of the test RAM region / 测试 RAM 区域基地址
const RAM_BASE: u32 = 0x2000_0000;

/// Size of the test RAM region / 测试 RAM 区域大小
const RAM_SIZE: usize = 0x1000;

/// `CTRL_TRIG.DATA_SIZE` = word / `CTRL_TRIG.DATA_SIZE` = 字
const DATA_SIZE_WORD: u32 = 2;

/// Word-copy control word with the permanent DREQ / 使用永久 DREQ 的字拷贝控制字
fn word_copy_ctrl() -> u32 {
    CTRL_EN
        | CTRL_INCR_READ
        | CTRL_INCR_WRITE
        | (DATA_SIZE_WORD << CTRL_DATA_SIZE_SHIFT)
        | (u32::from(TREQ_PERMANENT) << CTRL_TREQ_SEL_SHIFT)
}

/// Write one word to `bus` / 向 `bus` 写一个字
fn write_reg(bus: &mut SimpleBus, address: u32, value: u32) {
    let mut cycles = 0u64;
    bus.write_u32(address, value, &mut cycles).expect("write");
}

/// Read one word from `bus` / 从 `bus` 读一个字
fn read_reg(bus: &mut SimpleBus, address: u32) -> u32 {
    let mut cycles = 0u64;
    bus.read_u32(address, &mut cycles).expect("read")
}

// ---------------------------------------------------------------------------
// Priority hints / 优先级提示
// ---------------------------------------------------------------------------

#[test]
fn priority_fields_follow_the_datasheet() {
    let mut block = BusCtrl::new();
    assert_eq!(block.read32(BUS_PRIORITY), Some(0));
    // ACK 恒为 1：本模型立即锁存优先级。ACK reads 1: the model latches at once.
    assert_eq!(block.read32(BUS_PRIORITY_ACK), Some(1));
    assert!(block.write32(BUS_PRIORITY, DMA_R_PRIORITY_MASK | DMA_W_PRIORITY_MASK));
    assert_eq!(block.priority(), 0x1100);
    assert_eq!(block.read32(BUS_PRIORITY), Some(0x1100));
    // 保留位被丢弃；PROC0/PROC1/DMA_R/DMA_W 分别位于 bit0/4/8/12。
    // Reserved bits are dropped; PROC0/PROC1/DMA_R/DMA_W live at bit 0/4/8/12.
    assert!(block.write32(BUS_PRIORITY, 0xFFFF));
    assert_eq!(block.priority(), BUS_PRIORITY_MASK);
    assert_eq!(BUS_PRIORITY_MASK, 0x1111);
}

#[test]
fn alias_windows_update_the_priority_register() {
    let mut block = BusCtrl::new();
    assert!(block.write32(BUS_PRIORITY + 0x2000, DMA_W_PRIORITY_MASK));
    assert_eq!(block.priority(), DMA_W_PRIORITY_MASK);
    assert!(block.write32(BUS_PRIORITY + 0x3000, DMA_W_PRIORITY_MASK));
    assert_eq!(block.priority(), 0);
}

// ---------------------------------------------------------------------------
// Counter registers / 计数器寄存器
// ---------------------------------------------------------------------------

#[test]
fn perfctr_enable_is_stored() {
    let mut block = BusCtrl::new();
    // 只有 bit0 可写。Only bit 0 is writable.
    assert!(block.write32(PERFCTR_EN, 0xF));
    assert_eq!(block.read32(PERFCTR_EN), Some(1));
    block.reset();
    assert_eq!(block.read32(PERFCTR_EN), Some(0));
}

#[test]
fn perfsel_resets_to_sram6_access_and_masks_to_seven_bits() {
    let mut block = BusCtrl::new();
    // 复位值 0x1F 就是 sram6_access（端口 7，ACCESS 种类 3）。
    // The reset value 0x1F is sram6_access (port 7, ACCESS kind 3).
    assert_eq!(block.read32(PERFSEL0), Some(PERFSEL_RESET));
    assert_eq!(block.read32(PERFSEL1), Some(PERFSEL_RESET));
    assert_eq!(
        u32::from(BusPort::Sram(6).event_code(PERF_EVENT_ACCESS)),
        PERFSEL_RESET
    );
    assert!(block.write32(PERFSEL0, 0xFFFF_FFFF));
    assert_eq!(block.read32(PERFSEL0), Some(PERFSEL_MASK));
    // SET/CLR 别名同样折叠到 PERFSEL。The SET/CLR aliases fold onto PERFSEL too.
    assert!(block.write32(PERFSEL1 + 0x3000, 0x7F));
    assert_eq!(block.read32(PERFSEL1), Some(0));
    assert!(block.write32(PERFSEL1 + 0x2000, 0x03));
    assert_eq!(block.read32(PERFSEL1), Some(0x03));
}

#[test]
fn perfctr_write_clears_and_perfsel_change_keeps_the_count() {
    let perf = BusPerf::new();
    let mut block = BusCtrl::with_perf(perf.clone());
    assert!(block.write32(PERFCTR_EN, 1));
    assert!(block.write32(
        PERFSEL0,
        u32::from(BusPort::Sram(0).event_code(PERF_EVENT_ACCESS))
    ));
    perf.record_read(RAM_BASE, 1);
    perf.record_read(RAM_BASE + 4, 1);
    assert_eq!(block.read32(PERFCTR0), Some(2));
    // 数据手册只规定“写 PERFCTR 清零”，改选 PERFSEL 不清零。
    // Only a PERFCTR write clears; changing PERFSEL keeps the count.
    assert!(block.write32(
        PERFSEL0,
        u32::from(BusPort::Sram(1).event_code(PERF_EVENT_ACCESS))
    ));
    assert_eq!(block.read32(PERFCTR0), Some(2));
    perf.record_read(RAM_BASE + 8, 1);
    assert_eq!(
        block.read32(PERFCTR0),
        Some(2),
        "old event no longer counts"
    );
    // 写任意值清零。Any write clears.
    assert!(block.write32(PERFCTR0, 0xDEAD_BEEF));
    assert_eq!(block.read32(PERFCTR0), Some(0));
    // 另一个计数器不受影响。The other counter is independent.
    assert_eq!(block.read32(PERFCTR1), Some(0));
}

#[test]
fn perfctr_enable_gates_counting_and_has_alias_windows() {
    let perf = BusPerf::new();
    let mut block = BusCtrl::with_perf(perf.clone());
    assert!(block.write32(
        PERFSEL0,
        u32::from(BusPort::Sram(0).event_code(PERF_EVENT_ACCESS))
    ));
    // 停用时不计。Disabled counters do not increment.
    perf.record_read(RAM_BASE, 1);
    assert_eq!(block.read32(PERFCTR0), Some(0));
    // SET 别名置位 PERFCTR_EN。The SET alias sets PERFCTR_EN.
    assert!(block.write32(PERFCTR_EN + 0x2000, 1));
    assert_eq!(block.read32(PERFCTR_EN), Some(1));
    perf.record_read(RAM_BASE, 1);
    assert_eq!(block.read32(PERFCTR0), Some(1));
    // CLR 别名停用后再次冻结。The CLR alias freezes the counter again.
    assert!(block.write32(PERFCTR_EN + 0x3000, 1));
    perf.record_read(RAM_BASE, 1);
    assert_eq!(block.read32(PERFCTR0), Some(1));
}

#[test]
fn perfctr_saturates_at_24_bits() {
    let perf = BusPerf::new();
    let mut block = BusCtrl::with_perf(perf.clone());
    assert!(block.write32(PERFCTR_EN, 1));
    assert!(block.write32(
        PERFSEL0,
        u32::from(BusPort::Sram(0).event_code(PERF_EVENT_STALL_UPSTREAM))
    ));
    // 一个周期数超出 24 位的访问直接饱和，不再回绕。
    // One access with more cycles than 24 bits saturates instead of wrapping.
    perf.record_read(RAM_BASE, u64::from(u32::MAX));
    assert_eq!(block.read32(PERFCTR0), Some(PERFCTR_MASK));
    perf.record_read(RAM_BASE, 1);
    assert_eq!(block.read32(PERFCTR0), Some(PERFCTR_MASK));
    assert_eq!(PERFCTR_MASK, 0x00FF_FFFF);
}

// ---------------------------------------------------------------------------
// Port decode and event encoding / 端口解码与事件编码
// ---------------------------------------------------------------------------

#[test]
fn port_of_address_decodes_the_rp2350_map() {
    assert_eq!(port_of_address(0x0000_0000), Some(BusPort::Rom));
    assert_eq!(port_of_address(0x2000_0000), Some(BusPort::Sram(0)));
    assert_eq!(port_of_address(0x2001_2345), Some(BusPort::Sram(1)));
    assert_eq!(port_of_address(0x2007_FFFF), Some(BusPort::Sram(7)));
    assert_eq!(port_of_address(0x2008_0000), Some(BusPort::Sram(8)));
    assert_eq!(port_of_address(0x2008_1000), Some(BusPort::Sram(9)));
    // 520 KiB 窗口之外不属于任何 SRAM 端口。Outside the 520 KiB window.
    assert_eq!(port_of_address(0x2008_2000), None);
    assert_eq!(port_of_address(0x1000_0000), Some(BusPort::XipMain0));
    assert_eq!(port_of_address(0x1100_0000), Some(BusPort::XipMain1));
    assert_eq!(port_of_address(0x4006_8000), Some(BusPort::Apb));
    assert_eq!(port_of_address(0x5000_0000), Some(BusPort::FastPeri));
    // 未建模区域完全不计数。Unmodelled regions are not counted at all.
    assert_eq!(port_of_address(0xE000_0000), None);
}

#[test]
fn event_codes_follow_the_datasheet_order() {
    // 数据手册 PERFSEL 表：0x0B apb_access、0x37 sram0_access、0x43 rom_access。
    // Datasheet PERFSEL table: 0x0B apb_access, 0x37 sram0_access, 0x43 rom.
    assert_eq!(BusPort::Apb.event_code(PERF_EVENT_ACCESS), 0x0B);
    assert_eq!(BusPort::Sram(0).event_code(PERF_EVENT_ACCESS), 0x37);
    assert_eq!(BusPort::Rom.event_code(PERF_EVENT_ACCESS), 0x43);
    assert_eq!(
        BusPort::SiobProc0.event_code(PERF_EVENT_ACCESS_CONTESTED),
        0x06
    );
    assert_eq!(
        BusPort::FastPeri.event_code(PERF_EVENT_STALL_UPSTREAM),
        0x0C
    );
    // 端口序号与端口枚举可往返。Port indices round-trip.
    for index in 0..PORT_COUNT {
        let port = BusPort::from_index(index).expect("port");
        assert_eq!(port.index(), index);
    }
    assert_eq!(BusPort::from_index(PORT_COUNT), None);
}

#[test]
fn access_and_stall_events_count_per_port() {
    let perf = BusPerf::new();
    perf.set_enabled(true);
    perf.set_selector(0, BusPort::Sram(0).event_code(PERF_EVENT_ACCESS));
    perf.set_selector(1, BusPort::Sram(0).event_code(PERF_EVENT_STALL_UPSTREAM));
    perf.record_read(RAM_BASE, 3);
    perf.record_write(RAM_BASE + 4, 2);
    assert_eq!(perf.counter(0), 2, "ACCESS counts accesses");
    assert_eq!(perf.counter(1), 5, "STALL_UPSTREAM counts cycles");
    // 其它端口的访问不进入 sram0 计数器。Another port contributes nothing.
    perf.record_read(RAM_BASE + 0x1_0000, 7);
    assert_eq!(perf.counter(0), 2);
    assert_eq!(perf.counter(1), 5);
    let snapshot = perf.snapshot();
    assert_eq!(snapshot.dma_read_units, 2);
    assert_eq!(snapshot.dma_write_units, 1);
    assert_eq!(snapshot.dma_cycles, 12);
    assert_eq!(snapshot.port_accesses[BusPort::Sram(0).index()], 2);
    assert_eq!(snapshot.port_cycles[BusPort::Sram(0).index()], 5);
    assert_eq!(snapshot.port_accesses[BusPort::Sram(1).index()], 1);
}

#[test]
fn unobservable_events_stay_zero() {
    let perf = BusPerf::new();
    perf.set_enabled(true);
    perf.set_selector(0, BusPort::Sram(0).event_code(PERF_EVENT_ACCESS_CONTESTED));
    perf.set_selector(1, BusPort::Sram(0).event_code(PERF_EVENT_STALL_DOWNSTREAM));
    perf.record_read(RAM_BASE, 5);
    // 争用与下游 stall 需要周期精确仲裁器，保持 0。
    // Contention and downstream stalls need a cycle-exact arbiter: stay 0.
    assert_eq!(perf.snapshot().counters, [0, 0]);
    // 端口访问总计仍然可见。The port totals are still observable.
    assert_eq!(perf.snapshot().port_accesses[BusPort::Sram(0).index()], 1);
}

#[test]
fn occupancy_reports_parts_per_thousand() {
    let perf = BusPerf::new();
    perf.record_read(RAM_BASE, 250);
    assert_eq!(perf.occupancy(0), 0);
    assert_eq!(perf.occupancy(1000), 250);
    // DMA 周期超过窗口时饱和到 1000。Saturates when DMA cycles exceed the window.
    assert_eq!(perf.occupancy(100), 1000);
}

#[test]
fn reset_clears_counter_registers_but_keeps_accounting() {
    let perf = BusPerf::new();
    let mut block = BusCtrl::with_perf(perf.clone());
    assert!(block.write32(PERFCTR_EN, 1));
    perf.record_read(RAM_BASE, 4);
    assert_eq!(block.dma_bus_cycles(), 4);
    assert_eq!(block.dma_grants(), 0);
    block.reset();
    assert_eq!(block.read32(PERFCTR_EN), Some(0));
    assert_eq!(block.read32(PERFSEL0), Some(PERFSEL_RESET));
    assert_eq!(block.read32(PERFCTR0), Some(0));
    // 总线矩阵记账属于宿主视角，设备复位不清除。
    // Fabric accounting is host-side and survives a device reset.
    assert_eq!(block.dma_bus_cycles(), 4);
}

// ---------------------------------------------------------------------------
// DMA ↔ BUSCTRL wiring / DMA 与 BUSCTRL 接线
// ---------------------------------------------------------------------------

/// Fixture with RAM, a wired `BusCtrl` and a wired `Dma`
/// 带 RAM、已接线的 `BusCtrl` 与 `Dma` 的夹具
struct Wired {
    bus: SimpleBus,
    dma: Dma,
    ctrl: BusCtrl,
}

impl Wired {
    fn new() -> Self {
        let mut bus = SimpleBus::new();
        bus.register_region(Box::new(RamRegion::new("ram", RAM_BASE, RAM_SIZE)))
            .expect("ram region");
        let irq = IrqAssert::new();
        let perf = BusPerf::new();
        let ctrl = BusCtrl::with_perf(perf.clone());
        let dma = Dma::with_perf(irq, perf);
        mount_device(&mut bus, Box::new(ctrl.clone())).expect("busctrl window");
        mount_device(&mut bus, Box::new(dma.clone())).expect("dma window");
        Self { bus, dma, ctrl }
    }

    /// Program a word-copy channel and run it / 编程字拷贝通道并执行
    fn copy(&mut self, words: u32) {
        let base = map::DMA_BASE;
        write_reg(&mut self.bus, base, RAM_BASE);
        write_reg(&mut self.bus, base + 0x04, RAM_BASE + 0x100);
        write_reg(&mut self.bus, base + 0x08, words);
        write_reg(&mut self.bus, base + 0x0C, word_copy_ctrl());
        assert_eq!(
            self.dma.run(&mut self.bus, u64::from(words)),
            u64::from(words)
        );
    }
}

#[test]
fn dma_accesses_feed_the_busctrl_counters() {
    let mut wired = Wired::new();
    // 选择 sram0_access：读写地址都在 SRAM 0 号存储体。
    // Select sram0_access; both addresses live in SRAM bank 0.
    let access = u32::from(BusPort::Sram(0).event_code(PERF_EVENT_ACCESS));
    write_reg(&mut wired.bus, map::BUSCTRL_BASE + PERFSEL0, access);
    write_reg(&mut wired.bus, map::BUSCTRL_BASE + PERFSEL1, access);
    write_reg(&mut wired.bus, map::BUSCTRL_BASE + PERFCTR_EN, 1);
    wired.copy(4);
    // 4 个单元 = 4 次读 + 4 次写，全部落在 sram0。
    // 4 units = 4 reads + 4 writes, all attributed to sram0.
    let base = map::BUSCTRL_BASE;
    assert_eq!(read_reg(&mut wired.bus, base + PERFCTR0), 8);
    assert_eq!(read_reg(&mut wired.bus, base + PERFCTR1), 8);
    let counters = wired.dma.bus_counters();
    assert_eq!(counters.read_units, 4);
    assert_eq!(counters.write_units, 4);
    assert_eq!(counters.grants, 4);
    assert!(counters.read_cycles > 0 && counters.write_cycles > 0);
    assert_eq!(wired.ctrl.dma_grants(), 4, "grants reach BUSCTRL");
    // 写 PERFCTR 清零，之后的搬运重新开始计数。
    // A PERFCTR write clears; later transfers start counting again.
    write_reg(&mut wired.bus, base + PERFCTR0, 1);
    assert_eq!(read_reg(&mut wired.bus, base + PERFCTR0), 0);
    wired.copy(2);
    assert_eq!(read_reg(&mut wired.bus, base + PERFCTR0), 4);
}

#[test]
fn boot_peripherals_mount_busctrl_and_report_dma_counters() {
    let mut bus = SimpleBus::new();
    bus.register_region(Box::new(RamRegion::new("ram", RAM_BASE, RAM_SIZE)))
        .expect("ram region");
    let mut peripherals = BootPeripherals::mount(&mut bus).expect("mount");
    // BUSCTRL 挂在 APB：PERFSEL 复位 0x1F、PERFCTR 0、ACK 恒 1。
    // BUSCTRL is mounted in APB: PERFSEL resets to 0x1F, PERFCTR to 0, ACK 1.
    let base = map::BUSCTRL_BASE;
    assert_eq!(read_reg(&mut bus, base + PERFSEL0), PERFSEL_RESET);
    assert_eq!(read_reg(&mut bus, base + PERFCTR0), 0);
    assert_eq!(read_reg(&mut bus, base + BUS_PRIORITY_ACK), 1);
    assert_eq!(read_reg(&mut bus, base + PERFCTR_EN), 0);
    write_reg(
        &mut bus,
        base + PERFSEL0,
        u32::from(BusPort::Sram(0).event_code(PERF_EVENT_ACCESS)),
    );
    write_reg(&mut bus, base + PERFCTR_EN, 1);
    assert_eq!(read_reg(&mut bus, base + PERFSEL0), 0x37);
    assert_eq!(read_reg(&mut bus, base + PERFCTR_EN), 1);
    // `BootPeripherals` 的 DMA 至少报告本主设备的占用统计。
    // The BootPeripherals DMA at least reports this master's occupancy totals.
    let dma_base = map::DMA_BASE;
    write_reg(&mut bus, dma_base, RAM_BASE);
    write_reg(&mut bus, dma_base + 0x04, RAM_BASE + 0x100);
    write_reg(&mut bus, dma_base + 0x08, 4);
    write_reg(&mut bus, dma_base + 0x0C, word_copy_ctrl());
    assert_eq!(peripherals.dma_mut().run(&mut bus, 4), 4);
    let counters = peripherals.dma().bus_counters();
    assert_eq!(counters.read_units, 4);
    assert_eq!(counters.write_units, 4);
    assert_eq!(counters.grants, 4);
    assert!(counters.read_cycles > 0 && counters.write_cycles > 0);
}
