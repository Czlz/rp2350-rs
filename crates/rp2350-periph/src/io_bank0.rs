//! IO_BANK0 with pin function selection (`FUNCSEL`) / IO_BANK0 引脚功能选择（`FUNCSEL`）
//!
//! Every bank-0 GPIO exposes a read-only `GPIOx_STATUS` word at `0x000 + 8*n`
//! and a writable `GPIOx_CTRL` word at `0x004 + 8*n`. `CTRL.FUNCSEL`
//! (bits `[4:0]`) names the peripheral that owns the pin; it resets to `0x1F`
//! (`NULL`), matching the RP2350 datasheet. `FUNCSEL` values are decoded into
//! [`PinFunction`], and `STATUS.INFROMPAD` (bit 17) is driven live from the
//! shared [`GpioPads`] electrical model so a guest can observe the pad level.
//! The pads are the same handle `PADS_BANK0` and `SIO` use.
//! bank0 的每个 GPIO 在 `0x000 + 8*n` 处暴露只读的 `GPIOx_STATUS`，在
//! `0x004 + 8*n` 处暴露可写的 `GPIOx_CTRL`。`CTRL.FUNCSEL`（bits `[4:0]`）指明
//! 拥有该引脚的外设，复位值为 `0x1F`（`NULL`），与 RP2350 数据手册一致。
//! `FUNCSEL` 会解码为 [`PinFunction`]；`STATUS.INFROMPAD`（第 17 位）实时取自
//! 共享的 [`GpioPads`] 电气模型，因此客户机能观察到 pad 电平。该 pad 句柄与
//! `PADS_BANK0`、`SIO` 使用同一个。
//!
//! `FUNCSEL` is routed through the shared pin matrix: every `CTRL` write also
//! publishes the field into [`GpioPads`], where `SIO` and the `PIO` blocks only
//! drive/read pads they were selected for. `HSTX`/`SPI`/`UART`/`I2C`/`PWM`/
//! `USB`/`GPCK` have no modelled pad driver yet, so those pads keep their pull
//! or host-driven external level. Interrupt routing (`PROC0_*`/`PROC1_*`) and
//! `STATUS.OETOPAD`/`OUTTOPAD` are still stored only.
//! `FUNCSEL` 经共享引脚矩阵路由：每次写 `CTRL` 都会把该字段发布到 [`GpioPads`]，
//! `SIO` 与 `PIO` 块只驱动/读取自己被选中的 pad。`HSTX`/`SPI`/`UART`/`I2C`/
//! `PWM`/`USB`/`GPCK` 尚未建模 pad 驱动器，因此这些 pad 保持上下拉或宿主驱动的
//! 外部电平。中断路由（`PROC0_*`/`PROC1_*`）与 `STATUS.OETOPAD`/`OUTTOPAD`
//! 仍仅为存储。

use rp2350_core::mmio::{MmioDevice, MmioWindow, RegisterFile};

use super::gpio_pads::{FUNCSEL_MASK, FUNCSEL_NULL, GpioPads, PAD_COUNT};
use super::map;

/// Offset of the first `GPIOx_STATUS` register / 首个 `GPIOx_STATUS` 寄存器偏移
const STATUS_BASE: u32 = 0x000;

/// Offset of the first `GPIOx_CTRL` register / 首个 `GPIOx_CTRL` 寄存器偏移
const CTRL_BASE: u32 = 0x004;

/// Offset stride between two consecutive pins / 相邻两个引脚之间的偏移步长
const PIN_STRIDE: u32 = 8;

/// `FUNCSEL` reset value (`GPIO_FUNC_NULL`) / `FUNCSEL` 复位值（`GPIO_FUNC_NULL`）
const FUNCSEL_RESET: u32 = FUNCSEL_NULL;

/// `STATUS.INFROMPAD`: pad input level before override / 覆盖前的 pad 输入电平
const STATUS_INFROMPAD: u32 = 1 << 17;

/// Pin function selected by `GPIOx_CTRL.FUNCSEL` / `GPIOx_CTRL.FUNCSEL` 选择的引脚功能
///
/// The numbered variants are the RP2350 `gpio_function_rp2350` encodings. The
/// `Unknown` variant keeps an undocumented 5-bit value observable so a host can
/// tell that firmware programmed something the model does not recognise.
/// 带编号的变体对应 RP2350 的 `gpio_function_rp2350` 编码。`Unknown` 变体保留
/// 未在文档中出现的 5 位值，使宿主能看出固件写入了模型不认识的功能号。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PinFunction {
    /// HSTX / HSTX（TMDS 输出）
    Hstx,
    /// SPI / SPI
    Spi,
    /// UART0 / UART0
    Uart,
    /// I2C / I2C
    I2c,
    /// PWM / PWM
    Pwm,
    /// SIO (the CPU's own GPIO block) / SIO（CPU 自带的 GPIO 块）
    Sio,
    /// PIO0 / PIO0
    Pio0,
    /// PIO1 / PIO1
    Pio1,
    /// PIO2 / PIO2
    Pio2,
    /// GPCK (clock output), XIP CS1 or core-sight trace / GPCK（时钟输出），或 XIP CS1 / CoreSight trace
    Gpck,
    /// USB / USB
    Usb,
    /// UART1 (`UART_AUX`) / UART1（`UART_AUX`）
    UartAux,
    /// `NULL`: no peripheral owns the pin (reset value) / `NULL`：无外设拥有该引脚（复位值）
    Null,
    /// Undocumented 5-bit value `12..=30` / 未文档化的 5 位值 `12..=30`
    Unknown(u32),
}

impl PinFunction {
    /// Decode a `FUNCSEL` field value / 解码 `FUNCSEL` 字段值
    ///
    /// Values `0..=11` and `0x1F` map to their named variant, `12..=30` map to
    /// [`PinFunction::Unknown`] and `None` is returned for values that do not fit
    /// the 5-bit field (above `0x1F`).
    /// `0..=11` 与 `0x1F` 映射到具名变体，`12..=30` 映射到
    /// [`PinFunction::Unknown`]；超出 5 位字段（大于 `0x1F`）的值返回 `None`。
    #[must_use]
    pub const fn from_bits(bits: u32) -> Option<Self> {
        Some(match bits {
            0x00 => Self::Hstx,
            0x01 => Self::Spi,
            0x02 => Self::Uart,
            0x03 => Self::I2c,
            0x04 => Self::Pwm,
            0x05 => Self::Sio,
            0x06 => Self::Pio0,
            0x07 => Self::Pio1,
            0x08 => Self::Pio2,
            0x09 => Self::Gpck,
            0x0A => Self::Usb,
            0x0B => Self::UartAux,
            0x1F => Self::Null,
            0x0C..=0x1E => Self::Unknown(bits),
            _ => return None,
        })
    }

    /// Encode back to the `FUNCSEL` field value / 编码回 `FUNCSEL` 字段值
    ///
    /// [`PinFunction::Unknown`] returns the value it carries; callers that write
    /// the field mask it to 5 bits.
    /// [`PinFunction::Unknown`] 返回其携带的值；写入字段的调用方会将其掩码到 5 位。
    #[must_use]
    pub const fn to_bits(self) -> u32 {
        match self {
            Self::Hstx => 0x00,
            Self::Spi => 0x01,
            Self::Uart => 0x02,
            Self::I2c => 0x03,
            Self::Pwm => 0x04,
            Self::Sio => 0x05,
            Self::Pio0 => 0x06,
            Self::Pio1 => 0x07,
            Self::Pio2 => 0x08,
            Self::Gpck => 0x09,
            Self::Usb => 0x0A,
            Self::UartAux => 0x0B,
            Self::Null => 0x1F,
            Self::Unknown(bits) => bits,
        }
    }
}

/// IO_BANK0 register block / IO_BANK0 寄存器块
#[derive(Debug)]
pub struct IoBank0 {
    regs: RegisterFile,
    pads: GpioPads,
}

impl IoBank0 {
    /// Create the block with its own pad state / 创建使用独立 pad 状态的寄存器块
    ///
    /// Every `GPIOx_CTRL` resets to `FUNCSEL = NULL`; do not rely on a zeroed
    /// register file.
    /// 每个 `GPIOx_CTRL` 都复位为 `FUNCSEL = NULL`，不依赖寄存器文件清零。
    #[must_use]
    pub fn new() -> Self {
        Self::with_pads(GpioPads::new())
    }

    /// Create the block sharing electrical state with `PADS_BANK0`/`SIO`
    /// 创建与 `PADS_BANK0`/`SIO` 共享电气状态的寄存器块
    ///
    /// The pads handle is the same one passed to
    /// [`crate::pads_bank0::PadsBank0::with_pads`] and `Sio`, so
    /// `STATUS.INFROMPAD` and the SIO input register always agree.
    /// 该 pad 句柄与传给 [`crate::pads_bank0::PadsBank0::with_pads`] 及 `Sio` 的
    /// 句柄相同，因此 `STATUS.INFROMPAD` 与 SIO 输入寄存器始终一致。
    #[must_use]
    pub fn with_pads(pads: GpioPads) -> Self {
        let mut regs = RegisterFile::new(map::DEVICE_WINDOW_SIZE);
        for pin in 0..PAD_COUNT {
            regs.fill(ctrl_offset(pin), FUNCSEL_RESET);
            // 新句柄可能复用带旧 `FUNCSEL` 的 pad 矩阵，构造时统一置为 NULL。
            // A reused pad handle may still hold old `FUNCSEL` values, so the
            // constructor resets every pin to NULL.
            pads.set_funcsel(pin, FUNCSEL_RESET);
        }
        Self { regs, pads }
    }

    /// Shared pad matrix this block publishes its `FUNCSEL` view into
    /// 本块发布 `FUNCSEL` 视图所用的共享 pad 矩阵
    ///
    /// The handle is the same one `PADS_BANK0`, `SIO` and the `PIO` blocks use,
    /// so [`IoBank0::set_funcsel`] immediately changes which peripheral drives
    /// or samples a pad.
    /// 该句柄与 `PADS_BANK0`、`SIO` 及 `PIO` 块使用的相同，因此
    /// [`IoBank0::set_funcsel`] 会立即改变哪个外设驱动或采样某个 pad。
    #[must_use]
    pub fn pads(&self) -> &GpioPads {
        &self.pads
    }

    /// Bitmap of pads whose `FUNCSEL` selects `function` / `FUNCSEL` 选中 `function` 的 pad 位图
    ///
    /// Bit `n` is set when `GPIO{n}_CTRL.FUNCSEL` equals `function.to_bits()`;
    /// the result is the same mask the shared [`GpioPads`] routing uses.
    /// 当 `GPIO{n}_CTRL.FUNCSEL` 等于 `function.to_bits()` 时置位位 `n`；结果与共享
    /// [`GpioPads`] 路由使用的掩码一致。
    #[must_use]
    pub fn pins_for(&self, function: PinFunction) -> u64 {
        let bits = function.to_bits();
        let mut mask = 0u64;
        for pin in 0..PAD_COUNT {
            let ctrl = self.regs.read(ctrl_offset(pin)).unwrap_or(FUNCSEL_RESET);
            if ctrl & FUNCSEL_MASK == bits {
                mask |= 1u64 << pin;
            }
        }
        mask
    }

    /// Function selected for `pin`, or `None` outside bank 0
    /// `pin` 选择的功能；超出 bank0 范围时返回 `None`
    ///
    /// Undocumented `FUNCSEL` values are reported as
    /// [`PinFunction::Unknown`] rather than `None`, so callers can distinguish
    /// "pin does not exist" from "firmware programmed an undocumented value".
    /// 未文档化的 `FUNCSEL` 值以 [`PinFunction::Unknown`] 报告而非 `None`，便于
    /// 调用方区分「引脚不存在」与「固件写入了未文档化值」。
    #[must_use]
    pub fn funcsel(&self, pin: u32) -> Option<PinFunction> {
        if pin >= PAD_COUNT {
            return None;
        }
        let ctrl = self.regs.read(ctrl_offset(pin))?;
        PinFunction::from_bits(ctrl & FUNCSEL_MASK)
    }

    /// Select the function of `pin`, keeping the other `CTRL` bits
    /// 选择 `pin` 的功能，保留 `CTRL` 的其它位
    ///
    /// Writes `CTRL.FUNCSEL` (bits `[4:0]`) only; `OUTOVER`/`INOVER`/`OEOVER`
    /// and the interrupt bits are preserved. Pins outside bank 0 are ignored.
    /// The new value is published into the shared [`GpioPads`] matrix, so the
    /// selected peripheral starts driving/sampling the pad immediately.
    /// 仅写入 `CTRL.FUNCSEL`（bits `[4:0]`）；`OUTOVER`/`INOVER`/`OEOVER` 与中断
    /// 位保持不变。超出 bank0 的引脚被忽略。新值会发布到共享 [`GpioPads`] 矩阵，
    /// 被选中的外设立即开始驱动/采样该 pad。
    pub fn set_funcsel(&mut self, pin: u32, function: PinFunction) {
        if pin >= PAD_COUNT {
            return;
        }
        let offset = ctrl_offset(pin);
        let current = self.regs.read(offset).unwrap_or(FUNCSEL_RESET);
        let value = (current & !FUNCSEL_MASK) | (function.to_bits() & FUNCSEL_MASK);
        let _ = self.write32(offset, value);
    }

    /// Report whether `offset` addresses a pin `STATUS` register
    /// 判断 `offset` 是否指向某个引脚的 `STATUS` 寄存器
    fn status_pin(offset: u32) -> Option<u32> {
        if offset % PIN_STRIDE == 0 {
            let pin = offset / PIN_STRIDE;
            if pin < PAD_COUNT {
                return Some(pin);
            }
        }
        None
    }

    /// Report whether `offset` addresses a pin `CTRL` register
    /// 判断 `offset` 是否指向某个引脚的 `CTRL` 寄存器
    fn ctrl_pin(offset: u32) -> Option<u32> {
        if offset % PIN_STRIDE == CTRL_BASE {
            let pin = (offset - CTRL_BASE) / PIN_STRIDE;
            if pin < PAD_COUNT {
                return Some(pin);
            }
        }
        None
    }
}

/// Offset of `GPIOx_CTRL` for `pin` / 引脚 `pin` 的 `GPIOx_CTRL` 偏移
const fn ctrl_offset(pin: u32) -> u32 {
    CTRL_BASE + pin * PIN_STRIDE
}

/// Offset of `GPIOx_STATUS` for `pin` / 引脚 `pin` 的 `GPIOx_STATUS` 偏移
const fn status_offset(pin: u32) -> u32 {
    STATUS_BASE + pin * PIN_STRIDE
}

impl Default for IoBank0 {
    fn default() -> Self {
        Self::new()
    }
}

impl MmioDevice for IoBank0 {
    fn name(&self) -> &str {
        "IO_BANK0"
    }

    fn window(&self) -> MmioWindow {
        MmioWindow::new(map::IO_BANK0_BASE, map::DEVICE_WINDOW_SIZE)
    }

    fn read32(&mut self, offset: u32) -> Option<u32> {
        if let Some(pin) = Self::status_pin(offset) {
            // 硬件只读，写入被忽略；读出时把 pad 电平叠加到 INFROMPAD。
            // The word is read-only in hardware and writes are dropped; reads
            // overlay the live pad level onto INFROMPAD.
            let stored = self.regs.read(status_offset(pin)).unwrap_or(0);
            let high = (self.pads.input_levels() >> pin) & 1 != 0;
            let flag = if high { STATUS_INFROMPAD } else { 0 };
            return Some((stored & !STATUS_INFROMPAD) | flag);
        }
        self.regs.read(offset)
    }

    fn write32(&mut self, offset: u32, value: u32) -> bool {
        if Self::status_pin(offset).is_some() {
            // STATUS 为只读寄存器：写入被确认但不改变任何位。
            // STATUS is read-only: the write is acknowledged but changes nothing.
            return true;
        }
        let written = self.regs.write(offset, value);
        if written {
            if let Some(pin) = Self::ctrl_pin(offset) {
                // `CTRL.FUNCSEL` 同时发布到共享 pad 矩阵，路由立即生效。
                // `CTRL.FUNCSEL` is also published into the shared pad matrix,
                // so the routing takes effect immediately.
                self.pads.set_funcsel(pin, value & FUNCSEL_MASK);
            }
        }
        written
    }

    fn reset(&mut self) {
        // 恢复 `CTRL.FUNCSEL = NULL` 并清掉其它位；pad 电气状态属于
        // `PADS_BANK0`，不在此复位，但 `FUNCSEL` 路由视图必须一并清空。
        // Restore `CTRL.FUNCSEL = NULL` and clear every other bit; the pad
        // electrical state belongs to `PADS_BANK0` and is not reset here, but
        // the `FUNCSEL` routing view must be cleared with it.
        self.regs.reset();
        for pin in 0..PAD_COUNT {
            self.regs.fill(ctrl_offset(pin), FUNCSEL_RESET);
            self.pads.set_funcsel(pin, FUNCSEL_RESET);
        }
    }

    fn irq_number(&self) -> Option<u16> {
        Some(map::IO_IRQ_BANK0)
    }
}
