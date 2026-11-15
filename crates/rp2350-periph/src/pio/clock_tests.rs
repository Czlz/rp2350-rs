//! PIO clock-divider behaviour tests / PIO 时钟分频行为测试
//!
//! Every host tick is one `clk_sys` cycle, so `SMx_CLKDIV` divides the system
//! clock. These tests cover the two properties the timing model must have:
//! the fractional divisor gates machine cycles, and writing `CLKDIV` restarts
//! the divider so the new value takes effect on the next clock edge.
//! 每个宿主 tick 是一个 `clk_sys` 周期，因此 `SMx_CLKDIV` 分频的是系统时钟。
//! 这些测试覆盖时序模型的两个性质：小数分频正确门控机器周期，以及写入
//! `CLKDIV` 会重启分频器，使新值在下一个时钟边沿生效。

use super::Pio;
use super::regs::*;
use rp2350_core::mmio::MmioDevice;

/// `CTRL.SM_ENABLE` for SM0 / SM0 的 `CTRL.SM_ENABLE`
const SM0_ENABLE: u32 = 1;

/// A one-instruction program that jumps to itself / 跳回自身的单指令程序
const SPIN: u16 = 0x0000;

/// Program and enable SM0 / 编程并使能 SM0
fn rig() -> Pio {
    let mut pio = Pio::new(0);
    assert!(pio.write32(INSTR_MEM0, u32::from(SPIN)));
    assert!(pio.write32(CTRL, SM0_ENABLE));
    pio
}

#[test]
fn a_fractional_divider_gates_machine_cycles() {
    let mut pio = rig();
    // CLKDIV = 2.0：每两个 PIO 时钟退休一条指令。
    // CLKDIV = 2.0: one instruction retires every two PIO clocks.
    assert!(pio.write32(SM0_CLKDIV, 2 << 16));
    assert!(!pio.step_sm(0), "first edge only fills the divider");
    assert!(pio.step_sm(0), "second edge retires");
    assert!(!pio.step_sm(0));
    assert!(pio.step_sm(0));
    // CLKDIV = 1.5：两拍退休一次，余数 0.5 结转到下一拍（两拍一次）。
    // CLKDIV = 1.5: retires after two edges on average with the half carried.
    assert!(pio.write32(SM0_CLKDIV, (1 << 16) | 0x8000));
    assert!(!pio.step_sm(0));
    assert!(pio.step_sm(0));
    assert!(pio.step_sm(0), "0.5 carry retires on the very next edge");
}

#[test]
fn writing_clkdiv_restarts_the_divider_immediately() {
    let mut pio = rig();
    // 先走到 4.0 分频的第三拍（相位 768 / 1024），尚未退休。
    // Advance to the third edge of a 4.0 divider (phase 768 / 1024).
    assert!(pio.write32(SM0_CLKDIV, 4 << 16));
    assert!(!pio.step_sm(0));
    assert!(!pio.step_sm(0));
    assert!(!pio.step_sm(0));
    // 改成 1.0：分频器立即重启，下一拍就退休，而不是补完旧相位。
    // Change to 1.0: the divider restarts, so the next edge retires instead of
    // finishing the old phase.
    assert!(pio.write32(SM0_CLKDIV, 1 << 16));
    assert!(pio.step_sm(0));
    // 再改回 4.0 同样立即生效：第一拍不退休。
    // Changing back to 4.0 is immediate too: the first edge does not retire.
    assert!(pio.write32(SM0_CLKDIV, 4 << 16));
    assert!(!pio.step_sm(0));
}

#[test]
fn the_reset_divider_is_one_system_cycle_per_instruction() {
    // 复位值 CLKDIV = 1.0：每个 clk_sys 周期退休一条指令。
    // Reset CLKDIV = 1.0: one instruction per clk_sys cycle.
    let mut pio = rig();
    assert_eq!(pio.read32(SM0_CLKDIV), Some(1 << 16));
    assert!(pio.step_sm(0));
    assert!(pio.step_sm(0));
}
