//! Register decoding and frequency formulas / 寄存器解码与频率公式
//!
//! Private to [`super`]; only [`ClockState`] is exported to the tree facade.
//! 仅供 [`super`] 使用；只有 [`ClockState`] 暴露给时钟树外观层。

use super::formula::{divide, warn_invalid_pll};
use super::{
    CHANNEL_ENABLE, CHANNEL_ENABLED, CLK_ADC, CLK_HSTX, CLK_PERI, CLK_REF, CLK_SYS, CLK_USB,
    CLOCK_COUNT, LPOSC_HZ, PllId, ROSC_RING_HZ, XOSC_HZ,
};

/// `PLL_CS.BYPASS` / `PLL_CS.BYPASS`
const CS_BYPASS: u32 = 1 << 8;

/// Writable `PLL_CS` fields (`REFDIV` + `BYPASS`); `LOCK`/`LOCK_N` are not
/// stored here. 可写的 `PLL_CS` 字段（`REFDIV` + `BYPASS`）。
const CS_WRITABLE: u32 = 0x0000_013F;

/// `PLL_CS.REFDIV` mask / `PLL_CS.REFDIV` 掩码
const CS_REFDIV: u32 = 0x3F;

/// `PLL_CS.LOCK` / `PLL_CS.LOCK`
pub(super) const CS_LOCK: u32 = 1 << 31;

/// Writable `PLL_PWR` bits / 可写的 `PLL_PWR` 位
const PWR_WRITABLE: u32 = 0x2D;

/// `PLL_PWR.PD` (whole PLL powerdown) / `PLL_PWR.PD`（整块掉电）
const PWR_PD: u32 = 1 << 0;

/// `PLL_FBDIV_INT` mask (12 bits) / `PLL_FBDIV_INT` 掩码（12 位）
const FBDIV_MASK: u32 = 0x0FFF;

/// Minimum legal feedback divider / 合法反馈分频下限
const FBDIV_MIN: u32 = 16;

/// Maximum legal feedback divider / 合法反馈分频上限
const FBDIV_MAX: u32 = 320;

/// Writable `PLL_PRIM` bits (`POSTDIV1[18:16]`, `POSTDIV2[14:12]`)
/// 可写的 `PLL_PRIM` 位（`POSTDIV1[18:16]`、`POSTDIV2[14:12]`）
const PRIM_WRITABLE: u32 = 0x0007_7000;

/// `XOSC_CTRL.ENABLE` value that turns the crystal on
/// 打开晶振的 `XOSC_CTRL.ENABLE` 编码
const XOSC_ENABLE: u32 = 0xFAB;

/// `XOSC_CTRL.ENABLE` value that turns the crystal off
/// 关闭晶振的 `XOSC_CTRL.ENABLE` 编码
const XOSC_DISABLE: u32 = 0xD1E;

/// `XOSC_STATUS.STABLE` / `XOSC_STATUS.STABLE`
const XOSC_STATUS_STABLE: u32 = 1 << 31;

/// `XOSC_STATUS.BADWRITE` (write 1 to clear) / `XOSC_STATUS.BADWRITE`（写 1 清除）
const XOSC_STATUS_BADWRITE: u32 = 1 << 24;

/// `XOSC_STATUS.ENABLED` / `XOSC_STATUS.ENABLED`
const XOSC_STATUS_ENABLED: u32 = 1 << 12;

/// Valid `XOSC_CTRL.FREQ_RANGE` 12-bit codes / 合法的 `XOSC_CTRL.FREQ_RANGE` 编码
const XOSC_RANGES: [u32; 4] = [0xAA0, 0xAA1, 0xAA2, 0xAA3];

/// `ROSC_CTRL.ENABLE` value that turns the ring oscillator on
/// 打开环形振荡器的 `ROSC_CTRL.ENABLE` 编码
const ROSC_ENABLE: u32 = 0xFAB;

/// `ROSC_CTRL.ENABLE` value that turns the ring oscillator off
/// 关闭环形振荡器的 `ROSC_CTRL.ENABLE` 编码
const ROSC_DISABLE: u32 = 0xD1E;

/// `ROSC_DIV` valid prefix (`0xAA00 + div`, `div <= 127`)
/// `ROSC_DIV` 合法前缀（`0xAA00 + div`，`div <= 127`）
const ROSC_DIV_PREFIX: u32 = 0xAA00;

/// `ROSC_STATUS.STABLE` / `ROSC_STATUS.STABLE`
const ROSC_STATUS_STABLE: u32 = 1 << 31;

/// `ROSC_STATUS.BADWRITE` (write 1 to clear) / `ROSC_STATUS.BADWRITE`（写 1 清除）
const ROSC_STATUS_BADWRITE: u32 = 1 << 24;

/// `ROSC_STATUS.DIV_RUNNING` / `ROSC_STATUS.DIV_RUNNING`
const ROSC_STATUS_DIV_RUNNING: u32 = 1 << 16;

/// `ROSC_STATUS.ENABLED` / `ROSC_STATUS.ENABLED`
const ROSC_STATUS_ENABLED: u32 = 1 << 12;

/// Register layout of one `CLOCKS` channel / 一个 `CLOCKS` 通道的寄存器布局
#[derive(Debug, Clone, Copy)]
struct ChannelSpec {
    /// Writable `CTRL` bits / `CTRL` 可写位
    ctrl_mask: u32,
    /// Writable `DIV` bits / `DIV` 可写位
    div_mask: u32,
    /// `SRC` field mask, zero when the slice has no glitchless mux
    /// `SRC` 字段掩码；没有无毛刺选择器的通道为 0
    src_mask: u32,
    /// `AUXSRC` field shift / `AUXSRC` 字段位移
    aux_shift: u32,
    /// `AUXSRC` field mask / `AUXSRC` 字段掩码
    aux_mask: u32,
    /// Width of `DIV.INT` in bits / `DIV.INT` 的位宽
    int_bits: u32,
    /// Width of `DIV.FRAC` in bits (zero when absent) / `DIV.FRAC` 位宽（无则为 0）
    frac_bits: u32,
}

/// `CHANNELS` table mirrored from `hardware/regs/clocks.h`
/// 与 `hardware/regs/clocks.h` 一致的通道表
///
/// `CTRL` writable masks: GPOUT `0x0013_1DE0` (no `ENABLED`), `REF` `0x63`,
/// `SYS` `0xE1`, every AUX-only slice `0xCE0`. `DIV` masks follow the field
/// widths declared in the header.
/// `CTRL` 可写掩码：GPOUT `0x0013_1DE0`（不含 `ENABLED`），`REF` `0x63`，
/// `SYS` `0xE1`，只有 AUX 的通道 `0xCE0`。`DIV` 掩码按头文件字段宽度。
const CHANNELS: [ChannelSpec; CLOCK_COUNT] = [
    ChannelSpec {
        ctrl_mask: 0x0013_1DE0,
        div_mask: 0xFFFF_FFFF,
        src_mask: 0,
        aux_shift: 5,
        aux_mask: 0x1F,
        int_bits: 16,
        frac_bits: 16,
    },
    ChannelSpec {
        ctrl_mask: 0x0013_1DE0,
        div_mask: 0xFFFF_FFFF,
        src_mask: 0,
        aux_shift: 5,
        aux_mask: 0x1F,
        int_bits: 16,
        frac_bits: 16,
    },
    ChannelSpec {
        ctrl_mask: 0x0013_1DE0,
        div_mask: 0xFFFF_FFFF,
        src_mask: 0,
        aux_shift: 5,
        aux_mask: 0x1F,
        int_bits: 16,
        frac_bits: 16,
    },
    ChannelSpec {
        ctrl_mask: 0x0013_1DE0,
        div_mask: 0xFFFF_FFFF,
        src_mask: 0,
        aux_shift: 5,
        aux_mask: 0x1F,
        int_bits: 16,
        frac_bits: 16,
    },
    ChannelSpec {
        ctrl_mask: 0x63,
        div_mask: 0x00FF_0000,
        src_mask: 0x3,
        aux_shift: 5,
        aux_mask: 0x3,
        int_bits: 8,
        frac_bits: 0,
    },
    ChannelSpec {
        ctrl_mask: 0xE1,
        div_mask: 0xFFFF_FFFF,
        src_mask: 0x1,
        aux_shift: 5,
        aux_mask: 0x7,
        int_bits: 16,
        frac_bits: 16,
    },
    ChannelSpec {
        ctrl_mask: 0xCE0,
        div_mask: 0x0003_0000,
        src_mask: 0,
        aux_shift: 5,
        aux_mask: 0x7,
        int_bits: 2,
        frac_bits: 0,
    },
    ChannelSpec {
        ctrl_mask: 0xCE0,
        div_mask: 0x0003_0000,
        src_mask: 0,
        aux_shift: 5,
        aux_mask: 0x7,
        int_bits: 2,
        frac_bits: 0,
    },
    ChannelSpec {
        ctrl_mask: 0xCE0,
        div_mask: 0x000F_0000,
        src_mask: 0,
        aux_shift: 5,
        aux_mask: 0x7,
        int_bits: 4,
        frac_bits: 0,
    },
    ChannelSpec {
        ctrl_mask: 0xCE0,
        div_mask: 0x000F_0000,
        src_mask: 0,
        aux_shift: 5,
        aux_mask: 0x7,
        int_bits: 4,
        frac_bits: 0,
    },
];

/// Raw register image of one `CLOCKS` channel / 一个 `CLOCKS` 通道的原始寄存器镜像
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct ChannelRegs {
    /// `CLK_*_CTRL` (already masked) / `CLK_*_CTRL`（已掩码）
    pub(super) ctrl: u32,
    /// `CLK_*_DIV` (already masked) / `CLK_*_DIV`（已掩码）
    pub(super) div: u32,
}

/// Raw `PLL_CS`/`PWR`/`FBDIV_INT`/`PRIM` image of one PLL
/// 一个 PLL 的 `PLL_CS`/`PWR`/`FBDIV_INT`/`PRIM` 原始镜像
#[derive(Debug, Clone, Copy)]
struct PllRegs {
    /// `PLL_CS` writable bits / `PLL_CS` 可写位
    cs: u32,
    /// `PLL_PWR` writable bits / `PLL_PWR` 可写位
    pwr: u32,
    /// `PLL_FBDIV_INT` / `PLL_FBDIV_INT`
    fbdiv: u32,
    /// `PLL_PRIM` writable bits / `PLL_PRIM` 可写位
    prim: u32,
}

impl PllRegs {
    /// Post-bootrom PLL configuration / bootrom 之后的 PLL 配置
    const fn new(refdiv: u32, fbdiv: u32, first: u32, second: u32) -> Self {
        Self {
            cs: refdiv & CS_REFDIV,
            // PWR.PD/VCOPD/POSTDIVPD/DSMPD 全部清零：PLL 已上电。
            // All PWR powerdown bits clear: the PLL is running.
            pwr: 0,
            fbdiv: fbdiv & FBDIV_MASK,
            prim: ((first & 0x7) << 16) | ((second & 0x7) << 12),
        }
    }
}

/// One oscillator's register image plus decoded enable state
/// 一个振荡器的寄存器镜像与解码后的使能状态
#[derive(Debug, Clone, Copy)]
pub(super) struct Oscillator {
    /// Stored `CTRL` (only valid field encodings are written back)
    /// 保存的 `CTRL`（只写回合法字段编码）
    pub(super) ctrl: u32,
    /// Nominal output frequency before any divider / 分频前的标称频率
    pub(super) hz: u32,
    /// Whether the oscillator is enabled / 振荡器是否已使能
    pub(super) enabled: bool,
    /// Latched `STATUS.BADWRITE` / 锁存的 `STATUS.BADWRITE`
    pub(super) badwrite: bool,
}

/// Mutable state behind [`super::ClockTree`] / [`super::ClockTree`] 背后的可变状态
#[derive(Debug, Clone)]
pub(super) struct ClockState {
    /// Crystal oscillator / 晶振
    pub(super) xtal: Oscillator,
    /// Ring oscillator / 环形振荡器
    pub(super) ring: Oscillator,
    /// `ROSC_DIV` raw value / `ROSC_DIV` 原始值
    pub(super) ring_div: u32,
    /// System PLL / 系统 PLL
    pll_sys: PllRegs,
    /// USB PLL / USB PLL
    pll_usb: PllRegs,
    /// Ten channel register images / 十个通道的寄存器镜像
    pub(super) channels: [ChannelRegs; CLOCK_COUNT],
}

impl ClockState {
    /// Bootrom-equivalent state, not the power-on reset state
    /// bootrom 完成后的等价状态，而非芯片上电复位值
    ///
    /// XOSC 12 MHz, PLL_SYS 12 MHz / 1 * 125 / 5 / 2 = 150 MHz,
    /// PLL_USB 12 MHz / 1 * 100 / 5 / 5 = 48 MHz, `CLK_REF` = XOSC,
    /// `CLK_SYS` = `PLL_SYS`, `CLK_PERI`/`CLK_HSTX` = `CLK_SYS`,
    /// `CLK_USB`/`CLK_ADC` = `PLL_USB`.
    /// XOSC 12 MHz，PLL_SYS 12 MHz / 1 × 125 / 5 / 2 = 150 MHz，
    /// PLL_USB 12 MHz / 1 × 100 / 5 / 5 = 48 MHz，`CLK_REF` = XOSC，
    /// `CLK_SYS` = `PLL_SYS`，`CLK_PERI`/`CLK_HSTX` = `CLK_SYS`，
    /// `CLK_USB`/`CLK_ADC` = `PLL_USB`。
    pub(super) fn bootrom() -> Self {
        let mut channels = [ChannelRegs::default(); CLOCK_COUNT];
        let unit_div = 1u32 << 16;
        for slice in &mut channels {
            slice.div = unit_div;
        }
        // CLK_REF_CTRL.SRC = 2 (xosc_clksrc).
        channels[CLK_REF].ctrl = 2;
        // CLK_SYS_CTRL: SRC = 1 (aux), AUXSRC = 0 (clksrc_pll_sys).
        channels[CLK_SYS].ctrl = 0x01;
        // PERI/HSTX/USB/ADC: ENABLE = 1; AUXSRC = 0.
        channels[CLK_PERI].ctrl = CHANNEL_ENABLE;
        channels[CLK_HSTX].ctrl = CHANNEL_ENABLE;
        channels[CLK_USB].ctrl = CHANNEL_ENABLE;
        channels[CLK_ADC].ctrl = CHANNEL_ENABLE;
        Self {
            xtal: Oscillator {
                ctrl: (XOSC_ENABLE << 12) | XOSC_RANGES[0],
                hz: XOSC_HZ,
                enabled: true,
                badwrite: false,
            },
            ring: Oscillator {
                // ROSC_CTRL 复位值就是 0xAA0（未编程的 FREQ_RANGE 编码）。
                // The ROSC_CTRL reset value is 0xAA0 (unprogrammed range code).
                ctrl: 0x0000_0AA0,
                hz: ROSC_RING_HZ,
                enabled: true,
                badwrite: false,
            },
            // ROSC_DIV 复位为 div = 32（0xAA00 + 32）。
            // ROSC_DIV resets to div = 32 (0xAA00 + 32).
            ring_div: ROSC_DIV_PREFIX | 32,
            pll_sys: PllRegs::new(1, 125, 5, 2),
            pll_usb: PllRegs::new(1, 100, 5, 5),
            channels,
        }
    }

    /// Decoded `CTRL` for one channel / 一个通道解码后的 `CTRL`
    pub(super) fn channel_ctrl(&self, index: usize) -> u32 {
        let Some(slice) = self.channels.get(index) else {
            return 0;
        };
        let Some(spec) = CHANNELS.get(index) else {
            return 0;
        };
        // 只有带 ENABLE 位的通道才有 ENABLED 状态位；REF/SYS 没有该字段。
        // Only slices with an ENABLE bit have ENABLED; REF/SYS have no such field.
        if spec.src_mask == 0 && slice.ctrl & CHANNEL_ENABLE != 0 {
            slice.ctrl | CHANNEL_ENABLED
        } else {
            slice.ctrl
        }
    }

    /// Crystal output, zero while disabled / 晶振输出，关闭时为 0
    fn xtal_hz(&self) -> u32 {
        if self.xtal.enabled { self.xtal.hz } else { 0 }
    }

    /// ROSC output after `ROSC_DIV` / `ROSC_DIV` 之后的 ROSC 输出
    pub(super) fn ring_hz(&self) -> u32 {
        if !self.ring.enabled {
            return 0;
        }
        let raw = self.ring_div & 0xFFFF;
        let divisor = if raw & 0xFF80 == ROSC_DIV_PREFIX {
            match raw & 0x7F {
                0 => 128,
                value => value,
            }
        } else {
            // 非法编码按数据手册回落到 128。Invalid codes fall back to 128.
            128
        };
        self.ring.hz / divisor
    }

    /// One PLL's raw register image / 某个 PLL 的原始寄存器镜像
    fn pll(&self, id: PllId) -> PllRegs {
        match id {
            PllId::Sys => self.pll_sys,
            PllId::Usb => self.pll_usb,
        }
    }

    /// Whether a PLL reports lock / PLL 是否报告锁定
    pub(super) fn pll_locked(&self, id: PllId) -> bool {
        let regs = self.pll(id);
        let refdiv = regs.cs & CS_REFDIV;
        let fbdiv = regs.fbdiv & FBDIV_MASK;
        regs.pwr & PWR_PD == 0 && refdiv != 0 && (FBDIV_MIN..=FBDIV_MAX).contains(&fbdiv)
    }

    /// PLL output frequency / PLL 输出频率
    pub(super) fn pll_hz(&self, id: PllId) -> u32 {
        let regs = self.pll(id);
        let refdiv = regs.cs & CS_REFDIV;
        let ref_hz = self.xtal_hz();
        if refdiv == 0 {
            warn_invalid_pll("REFDIV = 0");
            return 0;
        }
        if regs.cs & CS_BYPASS != 0 {
            // BYPASS 把 refdiv 之后的参考时钟送到输出。BYPASS feeds the
            // reference (after REFDIV) to the output.
            return ref_hz / refdiv;
        }
        if regs.pwr & PWR_PD != 0 {
            return 0;
        }
        let fbdiv = regs.fbdiv & FBDIV_MASK;
        if !(FBDIV_MIN..=FBDIV_MAX).contains(&fbdiv) {
            warn_invalid_pll("FBDIV_INT outside 16..=320");
            return 0;
        }
        let first = (regs.prim >> 16) & 0x7;
        let second = (regs.prim >> 12) & 0x7;
        if first == 0 || second == 0 {
            warn_invalid_pll("POSTDIV1/POSTDIV2 = 0");
            return 0;
        }
        let vco = (u64::from(ref_hz) / u64::from(refdiv)) * u64::from(fbdiv);
        let out = vco / u64::from(first) / u64::from(second);
        u32::try_from(out).unwrap_or(u32::MAX)
    }

    /// PLL reference tap used by `clksrc_pll_usb_primary_ref_opcg`
    /// `clksrc_pll_usb_primary_ref_opcg` 使用的 PLL 参考抽头
    fn pll_ref_hz(&self, id: PllId) -> u32 {
        let refdiv = self.pll(id).cs & CS_REFDIV;
        if refdiv == 0 {
            return 0;
        }
        self.xtal_hz() / refdiv
    }

    /// Frequency of one channel / 某个通道的频率
    pub(super) fn channel_hz(&self, index: usize, depth: u8) -> u32 {
        if index >= CLOCK_COUNT || depth > 4 {
            return 0;
        }
        let spec = CHANNELS[index];
        let ctrl = self.channels[index].ctrl;
        let src = if spec.src_mask == 0 {
            1
        } else {
            ctrl & spec.src_mask
        };
        // 无毛刺选择器的 SRC=1 表示辅助源，其余编码直接选振荡器。
        // SRC=1 on a glitchless mux selects the auxiliary source; other
        // encodings select an oscillator directly.
        let source = if spec.src_mask == 0 || src == 1 {
            self.aux_hz(index, (ctrl >> spec.aux_shift) & spec.aux_mask, depth)
        } else {
            match (index, src) {
                (CLK_REF, 0) => self.ring_hz(),
                (CLK_REF, 2) => self.xtal_hz(),
                (CLK_REF, 3) => LPOSC_HZ,
                (CLK_SYS, _) => self.channel_hz(CLK_REF, depth + 1),
                _ => 0,
            }
        };
        divide(
            source,
            self.channels[index].div,
            spec.int_bits,
            spec.frac_bits,
        )
    }

    /// Frequency of a channel's `AUXSRC` selection / 通道 `AUXSRC` 选择的频率
    fn aux_hz(&self, index: usize, aux: u32, depth: u8) -> u32 {
        match index {
            // CLK_REF aux: 0 PLL_USB, 1/2 GPIN, 3 PLL_USB primary ref.
            CLK_REF => match aux {
                0 => self.pll_hz(PllId::Usb),
                3 => self.pll_ref_hz(PllId::Usb),
                _ => 0,
            },
            // CLK_SYS aux: 0 PLL_SYS, 1 PLL_USB, 2 ROSC, 3 XOSC, 4/5 GPIN.
            CLK_SYS => match aux {
                0 => self.pll_hz(PllId::Sys),
                1 => self.pll_hz(PllId::Usb),
                2 => self.ring_hz(),
                3 => self.xtal_hz(),
                _ => 0,
            },
            // CLK_PERI aux: 0 CLK_SYS, 1 PLL_SYS, 2 PLL_USB, 3 ROSC, 4 XOSC.
            CLK_PERI => match aux {
                0 => self.channel_hz(CLK_SYS, depth + 1),
                1 => self.pll_hz(PllId::Sys),
                2 => self.pll_hz(PllId::Usb),
                3 => self.ring_hz(),
                4 => self.xtal_hz(),
                _ => 0,
            },
            // CLK_HSTX aux: 0 CLK_SYS, 1 PLL_SYS, 2 PLL_USB, 3/4 GPIN.
            CLK_HSTX => match aux {
                0 => self.channel_hz(CLK_SYS, depth + 1),
                1 => self.pll_hz(PllId::Sys),
                2 => self.pll_hz(PllId::Usb),
                _ => 0,
            },
            // CLK_USB / CLK_ADC aux: 0 PLL_USB, 1 PLL_SYS, 2 ROSC, 3 XOSC.
            CLK_USB | CLK_ADC => match aux {
                0 => self.pll_hz(PllId::Usb),
                1 => self.pll_hz(PllId::Sys),
                2 => self.ring_hz(),
                3 => self.xtal_hz(),
                _ => 0,
            },
            // CLK_GPOUTn aux (5 bits), see CLOCKS_CLK_GPOUT0_CTRL_AUXSRC.
            _ => match aux {
                0x0 => self.pll_hz(PllId::Sys),
                0x3 => self.pll_hz(PllId::Usb),
                0x4 => self.pll_ref_hz(PllId::Usb),
                0x5 => self.ring_hz(),
                0x6 => self.xtal_hz(),
                0x7 => LPOSC_HZ,
                0x8 => self.channel_hz(CLK_SYS, depth + 1),
                0x9 => self.channel_hz(CLK_USB, depth + 1),
                0xA => self.channel_hz(CLK_ADC, depth + 1),
                0xB => self.channel_hz(CLK_REF, depth + 1),
                0xC => self.channel_hz(CLK_PERI, depth + 1),
                0xD => self.channel_hz(CLK_HSTX, depth + 1),
                // 0x1/0x2 GPIN0/1、0xE OTP_CLK2FC 未建模。
                // 0x1/0x2 GPIN0/1 and 0xE OTP_CLK2FC are not modelled.
                _ => 0,
            },
        }
    }

    /// Masked `CTRL` write / 带掩码的 `CTRL` 写入
    pub(super) fn store_ctrl(&mut self, index: usize, value: u32) {
        if let (Some(spec), Some(slice)) = (CHANNELS.get(index), self.channels.get_mut(index)) {
            slice.ctrl = value & spec.ctrl_mask;
        }
    }

    /// Masked `DIV` write / 带掩码的 `DIV` 写入
    pub(super) fn store_div(&mut self, index: usize, value: u32) {
        if let (Some(spec), Some(slice)) = (CHANNELS.get(index), self.channels.get_mut(index)) {
            slice.div = value & spec.div_mask;
        }
    }

    /// Masked PLL register write; returns whether the offset is modelled
    /// 带掩码的 PLL 寄存器写入；返回偏移是否已建模
    pub(super) fn store_pll(&mut self, id: PllId, offset: u32, value: u32) -> bool {
        let regs = match id {
            PllId::Sys => &mut self.pll_sys,
            PllId::Usb => &mut self.pll_usb,
        };
        match offset {
            0x00 => regs.cs = value & CS_WRITABLE,
            0x04 => regs.pwr = value & PWR_WRITABLE,
            0x08 => regs.fbdiv = value & FBDIV_MASK,
            0x0C => regs.prim = value & PRIM_WRITABLE,
            _ => return false,
        }
        true
    }

    /// Restore one PLL to the bootrom image / 把一个 PLL 恢复到 bootrom 镜像
    pub(super) fn restore_pll(&mut self, id: PllId, boot: &Self) {
        match id {
            PllId::Sys => self.pll_sys = boot.pll_sys,
            PllId::Usb => self.pll_usb = boot.pll_usb,
        }
    }

    /// `PLL_CS` image / `PLL_CS` 镜像
    pub(super) fn pll_cs(&self, id: PllId) -> u32 {
        let regs = self.pll(id);
        if self.pll_locked(id) {
            regs.cs | CS_LOCK
        } else {
            regs.cs
        }
    }

    /// `PLL_PWR` image / `PLL_PWR` 镜像
    pub(super) fn pll_pwr(&self, id: PllId) -> u32 {
        self.pll(id).pwr
    }

    /// `PLL_FBDIV_INT` image / `PLL_FBDIV_INT` 镜像
    pub(super) fn pll_fbdiv(&self, id: PllId) -> u32 {
        self.pll(id).fbdiv
    }

    /// `PLL_PRIM` image / `PLL_PRIM` 镜像
    pub(super) fn pll_prim(&self, id: PllId) -> u32 {
        self.pll(id).prim
    }

    /// `XOSC_STATUS` value / `XOSC_STATUS` 值
    pub(super) fn xosc_status(&self) -> u32 {
        // STATUS.FREQ_RANGE 是 CTRL 编码在 0xAA0..=0xAA3 表中的索引。
        // STATUS.FREQ_RANGE indexes the range code stored in CTRL.
        let range = XOSC_RANGES
            .iter()
            .position(|code| *code == (self.xtal.ctrl & 0xFFF))
            .map_or(0, |index| index as u32);
        let mut status = range & 0x3;
        if self.xtal.enabled {
            status |= XOSC_STATUS_STABLE | XOSC_STATUS_ENABLED;
        }
        if self.xtal.badwrite {
            status |= XOSC_STATUS_BADWRITE;
        }
        status
    }

    /// Apply an `XOSC_CTRL` write / 应用 `XOSC_CTRL` 写入
    pub(super) fn write_xosc_ctrl(&mut self, value: u32) {
        let mut ctrl = self.xtal.ctrl;
        let mut bad = false;
        let enable = (value >> 12) & 0xFFF;
        match enable {
            XOSC_ENABLE => {
                ctrl = (ctrl & !0x00FF_F000) | (XOSC_ENABLE << 12);
                self.xtal.enabled = true;
            }
            XOSC_DISABLE => {
                ctrl = (ctrl & !0x00FF_F000) | (XOSC_DISABLE << 12);
                self.xtal.enabled = false;
            }
            _ => bad = true,
        }
        let range = value & 0xFFF;
        if let Some(code) = XOSC_RANGES.iter().find(|code| **code == range) {
            ctrl = (ctrl & !0xFFF) | code;
        } else {
            bad = true;
        }
        self.xtal.ctrl = ctrl;
        self.xtal.badwrite |= bad;
    }

    /// Apply an `ROSC_CTRL` write / 应用 `ROSC_CTRL` 写入
    pub(super) fn write_rosc_ctrl(&mut self, value: u32) {
        let mut ctrl = self.ring.ctrl;
        let mut bad = false;
        let enable = (value >> 12) & 0xFFF;
        match enable {
            ROSC_ENABLE => {
                ctrl = (ctrl & !0x00FF_F000) | (ROSC_ENABLE << 12);
                self.ring.enabled = true;
            }
            ROSC_DISABLE => {
                ctrl = (ctrl & !0x00FF_F000) | (ROSC_DISABLE << 12);
                self.ring.enabled = false;
            }
            _ => bad = true,
        }
        let range = value & 0xFFF;
        // FREQ_RANGE 合法编码：0xFA4 LOW / 0xFA5 MEDIUM / 0xFA7 HIGH /
        // 0xFA6 TOOHIGH；其余值保留原编码。
        // Valid FREQ_RANGE codes: 0xFA4 LOW / 0xFA5 MEDIUM / 0xFA7 HIGH /
        // 0xFA6 TOOHIGH; anything else keeps the previous code.
        if matches!(range, 0xFA4..=0xFA7) {
            ctrl = (ctrl & !0xFFF) | range;
        } else {
            bad = true;
        }
        self.ring.ctrl = ctrl;
        self.ring.badwrite |= bad;
    }

    /// `ROSC_STATUS` value / `ROSC_STATUS` 值
    pub(super) fn rosc_status(&self) -> u32 {
        let mut status = 0;
        if self.ring.enabled {
            status |= ROSC_STATUS_STABLE | ROSC_STATUS_DIV_RUNNING | ROSC_STATUS_ENABLED;
        }
        if self.ring.badwrite {
            status |= ROSC_STATUS_BADWRITE;
        }
        status
    }
}
