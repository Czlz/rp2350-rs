//! HSTX TMDS pixel capture / HSTX TMDS 像素捕获
//!
//! The command expander emits one 32-bit output word per `CMD_TMDS` data word.
//! With `EXPAND_SHIFT.ENC_SHIFT = N` a word carries `32 / N` pixels (one when
//! `N = 0`, where the word cannot be split), and `EXPAND_TMDS` splits each pixel
//! into three lane symbols: for lane `x` the word is right-rotated by `Lx_ROT`
//! and the symbol is the `Lx_NBITS + 1` bits anchored at bit 7 of the rotated
//! word ("starting from bit 7 of the rotated data ... remaining LSBs are masked
//! to 0", per the RP2350 register header).
//! 命令扩展器为每个 `CMD_TMDS` 数据字产生一个 32 位输出字。`ENC_SHIFT = N` 时
//! 每个字承载 `32 / N` 个像素（`N = 0` 时无法切分，按 1 个处理），
//! `EXPAND_TMDS` 把每个像素切成三路通道符号：通道 `x` 先把字右旋 `Lx_ROT` 位，
//! 符号取旋转后字中锚定在位 7 的 `Lx_NBITS + 1` 位（RP2350 寄存器头文件原文：
//! 从旋转后数据的位 7 开始，剩余低位清零）。
//!
//! These symbols are **pre-encoding** values: TMDS 10-bit coding, DC balancing
//! and DVI/HDMI bit timing are not modelled, so a host can rebuild RGB565 or
//! RGB888 from them but cannot recover the serialised TMDS stream.
//! 这些符号是**编码前**取值：TMDS 10 位编码、直流平衡与 DVI/HDMI 位时序均未
//! 建模，因此宿主可以据此重建 RGB565 或 RGB888，但无法还原串行化的 TMDS 码流。

use std::collections::VecDeque;

/// TMDS lanes per pixel: lane 0 = blue, lane 1 = green, lane 2 = red
/// 每个像素的 TMDS 通道数：通道 0 = 蓝、通道 1 = 绿、通道 2 = 红
pub const LANES: usize = 3;

/// `Lx_NBITS` position inside `EXPAND_TMDS` for lanes 0..2 (`[7:5]`, `[15:13]`, `[23:21]`)
/// `EXPAND_TMDS` 中通道 0..2 的 `Lx_NBITS` 位位置（`[7:5]`、`[15:13]`、`[23:21]`）
const NBITS_SHIFT: [u32; LANES] = [5, 13, 21];

/// `Lx_ROT` position inside `EXPAND_TMDS` for lanes 0..2 (`[4:0]`, `[12:8]`, `[20:16]`)
/// `EXPAND_TMDS` 中通道 0..2 的 `Lx_ROT` 位位置（`[4:0]`、`[12:8]`、`[20:16]`）
const ROT_SHIFT: [u32; LANES] = [0, 8, 16];

/// `Lx_NBITS` field mask (3 bits: values 0..=7 encode widths 1..=8)
/// `Lx_NBITS` 字段掩码（3 位：0..=7 表示宽度 1..=8）
const NBITS_MASK: u32 = 0x7;

/// `Lx_ROT` field mask / `Lx_ROT` 字段掩码
const ROT_MASK: u32 = 0x1f;

/// One captured pixel: three lane symbols before TMDS encoding
/// 一个捕获到的像素：TMDS 编码前的三路通道符号
///
/// `lanes[0]`/`lanes[1]`/`lanes[2]` are the blue/green/red symbols, each
/// right-aligned and `Lx_NBITS + 1` bits wide. No TMDS 10-bit coding, DC
/// balancing or DVI timing is applied.
/// `lanes[0]`/`lanes[1]`/`lanes[2]` 依次为蓝/绿/红符号，各自右对齐、宽度为
/// `Lx_NBITS + 1` 位。未施加 TMDS 10 位编码、直流平衡或 DVI 时序。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HstxPixel {
    /// Blue, green and red lane symbols / 蓝、绿、红通道符号
    pub lanes: [u8; LANES],
}

/// One entry of the host-side output queue / 宿主输出队列中的一项
#[derive(Debug, Clone, Copy)]
pub(crate) struct OutputWord {
    /// The 32-bit output word / 32 位输出字
    pub(crate) word: u32,
    /// `true` when the word was produced by a TMDS data command
    /// 该字由 TMDS 数据命令产生时为 `true`
    pub(crate) tmds: bool,
}

/// Pixels carried by one TMDS data word / 一个 TMDS 数据字承载的像素数
///
/// `ENC_SHIFT = 0` cannot split a word, so the model reports one pixel; `N = 0`
/// is documented as the fallback case of [`decode_word`].
/// `ENC_SHIFT = 0` 无法切分字，模型按 1 个像素处理；该情形在 [`decode_word`]
/// 中按回退路径记录。
#[must_use]
pub fn pixels_per_word(enc_shift: u32) -> u32 {
    (32u32.checked_div(enc_shift).unwrap_or(1)).max(1)
}

/// Extract one lane symbol from a word / 从一个字中提取一路通道符号
///
/// The symbol is the `nbits + 1` bits anchored at bit 7 of the rotated word,
/// exactly as the datasheet describes (`width = 1..=8`).
/// 符号取旋转后字中锚定在位 7 的 `nbits + 1` 位，与数据手册一致
/// （宽度 1..=8）。
fn lane_symbol(word: u32, rot: u32, nbits: u32) -> u8 {
    let width = (nbits + 1).min(8);
    let rotated = word.rotate_right(rot & ROT_MASK);
    // 手动 `try_from` 而不是 `as u8`：掩码已保证结果 <= 0xFF。
    // `try_from` instead of `as u8`: the mask already bounds the value.
    u8::try_from((rotated >> (8 - width)) & ((1 << width) - 1)).unwrap_or(0)
}

/// Decode one TMDS data word into its pixels / 把一个 TMDS 数据字解码为像素
///
/// Pixels are returned in transmission order: pixel 0 is the lowest
/// `ENC_SHIFT` bits, and each following pixel rotates the word one step further.
/// When the lane configuration cannot be decoded (`EXPAND_TMDS` is all zero, or
/// `ENC_SHIFT` is 0 so the word cannot be split) the fallback returns one
/// `HstxPixel` per slice with `lanes[0]` holding the low 8 bits of that slice
/// and `lanes[1]`/`lanes[2]` zero, so a host can still see the raw payload.
/// 像素按发送顺序返回：像素 0 取最低 `ENC_SHIFT` 位，后续像素依次再旋转一步。
/// 当通道配置无法可靠解码（`EXPAND_TMDS` 全 0，或 `ENC_SHIFT` 为 0 无法切分）时，
/// 回退路径为每个切片返回一个 `HstxPixel`：`lanes[0]` 为该切片的低 8 位，
/// `lanes[1]`/`lanes[2]` 为 0，宿主仍能看到原始载荷。
#[must_use]
pub fn decode_word(word: u32, expand_tmds: u32, enc_shift: u32) -> Vec<HstxPixel> {
    let per_word = pixels_per_word(enc_shift);
    let fallback = expand_tmds == 0 || enc_shift == 0;
    let mut pixels = Vec::with_capacity(usize::try_from(per_word).unwrap_or(1));
    for index in 0..per_word {
        let slice = if enc_shift == 0 {
            word
        } else {
            word.rotate_right(index * enc_shift)
        };
        if fallback {
            pixels.push(HstxPixel {
                lanes: [u8::try_from(slice & 0xff).unwrap_or(0), 0, 0],
            });
            continue;
        }
        let mut lanes = [0u8; LANES];
        for (lane, slot) in lanes.iter_mut().enumerate() {
            let nbits = (expand_tmds >> NBITS_SHIFT[lane]) & NBITS_MASK;
            let rot = (expand_tmds >> ROT_SHIFT[lane]) & ROT_MASK;
            *slot = lane_symbol(slice, rot, nbits);
        }
        pixels.push(HstxPixel { lanes });
    }
    pixels
}

/// Drain every TMDS word from the queue and decode it / 取走队列中所有 TMDS 字并解码
///
/// Non-TMDS words (raw mode words and `CMD_RAW`/`CMD_RAW_REPEAT` payloads) stay
/// in the queue for [`crate::hstx::Hstx::drain_output`], so a host can capture
/// pixels and still read the sync/control words.
/// 非 TMDS 字（原始模式字与 `CMD_RAW`/`CMD_RAW_REPEAT` 载荷）留在队列中，供
/// [`crate::hstx::Hstx::drain_output`] 取用，因此宿主既能捕获像素，也能读到
/// 同步/控制字。
pub(crate) fn drain_pixels(
    output: &mut VecDeque<OutputWord>,
    expand_tmds: u32,
    enc_shift: u32,
) -> Vec<HstxPixel> {
    let mut pixels = Vec::new();
    if !output.iter().any(|word| word.tmds) {
        return pixels;
    }
    let mut kept = VecDeque::with_capacity(output.len());
    for word in output.drain(..) {
        if word.tmds {
            pixels.extend(decode_word(word.word, expand_tmds, enc_shift));
        } else {
            kept.push_back(word);
        }
    }
    *output = kept;
    pixels
}
