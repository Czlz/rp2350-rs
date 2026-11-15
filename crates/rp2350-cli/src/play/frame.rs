//! Frame geometry, RGB565 decoding and the published frame type
//! 帧几何、RGB565 解码与发布帧类型
//!
//! A frame buffer is described by a [`FrameSpec`]: base address, size and row
//! stride. [`FrameSpec::decode`] turns the raw guest bytes into one `u16` per
//! pixel (RGB565, little-endian, as stored by the RP2350 firmware), skipping
//! any row padding when `stride_bytes > width * 2`.
//! 帧缓冲由 [`FrameSpec`] 描述：基地址、尺寸与行跨度。[`FrameSpec::decode`] 把
//! 原始客户机字节转换为每像素一个 `u16`（RGB565、小端，与 RP2350 固件的存储
//! 形式一致），并在 `stride_bytes > width * 2` 时跳过行填充。

// 帧坐标与字节偏移只在 16/32 位像素格式内换算，截断是像素格式本身的语义。
// Frame coordinates and byte offsets stay inside the 16/32-bit pixel format, so
// the narrowing conversions are inherent to that format.
#![allow(clippy::cast_possible_truncation)]

use std::sync::Arc;

/// Geometry and location of a guest frame buffer / 客户机帧缓冲的位置与几何
///
/// `stride_bytes` is the distance between two row starts. It defaults to
/// `width * 2` (packed RGB565) and may be larger when the firmware pads rows.
/// `stride_bytes` 是相邻两行起始地址之间的距离：默认为 `width * 2`（紧凑
/// RGB565），固件做行填充时可以更大。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameSpec {
    /// First byte of the frame buffer / 帧缓冲首字节地址
    pub base: u32,
    /// Pixels per row / 每行像素数
    pub width: u16,
    /// Number of rows / 行数
    pub height: u16,
    /// Bytes between two row starts / 相邻两行起始地址之间的字节数
    pub stride_bytes: u32,
}

impl FrameSpec {
    /// Build a packed RGB565 spec (`stride = width * 2`)
    /// 构造紧凑 RGB565 规格（`stride = width * 2`）
    #[must_use]
    pub fn new(base: u32, width: u16, height: u16) -> Self {
        Self {
            base,
            width,
            height,
            stride_bytes: u32::from(width) * 2,
        }
    }

    /// Parse `ADDR:WxH[:STRIDE]` / 解析 `ADDR:WxH[:STRIDE]`
    ///
    /// `ADDR` and `STRIDE` accept decimal or `0x`-prefixed hexadecimal and
    /// `W`/`H` are decimal pixel counts. The stride defaults to `W * 2` and must
    /// not be smaller, otherwise rows would overlap.
    /// `ADDR` 与 `STRIDE` 接受十进制或 `0x` 十六进制，`W`/`H` 为十进制像素数。
    /// 跨度默认 `W * 2`，且不得更小，否则行会重叠。
    ///
    /// # Errors
    /// Returns a message naming the malformed field or the violated constraint.
    /// 返回指出格式错误字段或约束冲突的消息。
    pub fn parse(spec: &str) -> Result<Self, String> {
        let mut fields = spec.split(':');
        let base = fields
            .next()
            .filter(|field| !field.is_empty())
            .ok_or_else(|| format!("missing address in `{spec}`"))?;
        let size = fields
            .next()
            .ok_or_else(|| format!("missing WxH in `{spec}`"))?;
        let stride = fields.next();
        if fields.next().is_some() {
            return Err(format!(
                "too many fields in `{spec}` (expected ADDR:WxH[:STRIDE]) / \
                 字段过多（应为 ADDR:WxH[:STRIDE]）"
            ));
        }
        let (width, height) = size
            .split_once(['x', 'X'])
            .ok_or_else(|| format!("bad size `{size}` in `{spec}` (expected WxH)"))?;
        let base = parse_number(base).ok_or_else(|| format!("bad address `{base}`"))?;
        let width: u16 = width
            .trim()
            .parse()
            .map_err(|_| format!("bad width `{width}` in `{spec}`"))?;
        let height: u16 = height
            .trim()
            .parse()
            .map_err(|_| format!("bad height `{height}` in `{spec}`"))?;
        if width == 0 || height == 0 {
            return Err(format!("empty frame `{spec}` / 帧尺寸为零"));
        }
        let packed = u32::from(width) * 2;
        let stride_bytes = match stride {
            // 允许 `:STRIDE` 但拒绝空字段，空跨度是笔误。 / An empty STRIDE is a typo.
            Some("") => {
                return Err(format!("empty stride in `{spec}` / 跨度为空白"));
            }
            Some(text) => {
                parse_number(text).ok_or_else(|| format!("bad stride `{text}` in `{spec}`"))?
            }
            None => packed,
        };
        if stride_bytes < packed {
            return Err(format!(
                "stride {stride_bytes} < width*2 ({packed}) in `{spec}` / 跨度小于行字节数"
            ));
        }
        Ok(Self {
            base,
            width,
            height,
            stride_bytes,
        })
    }

    /// Bytes the frame buffer occupies, padding included
    /// 帧缓冲占用的字节数（含行填充）
    #[must_use]
    pub fn buffer_len(&self) -> usize {
        usize::from(self.height) * self.stride_bytes as usize
    }

    /// Decode `bytes` into one little-endian RGB565 word per pixel
    /// 把 `bytes` 解码为每像素一个小端 RGB565 字
    ///
    /// Missing bytes decode as `0`, so an unreadable tail produces black pixels
    /// instead of a panic; row padding between `width * 2` and
    /// `stride_bytes` is skipped.
    /// 缺失的字节解码为 `0`，因此读不到的部分变成黑像素而不是 panic；`width * 2`
    /// 与 `stride_bytes` 之间的行填充被跳过。
    #[must_use]
    pub fn decode(&self, bytes: &[u8]) -> Vec<u16> {
        let width = usize::from(self.width);
        let height = usize::from(self.height);
        let stride = self.stride_bytes as usize;
        let mut pixels = Vec::with_capacity(width * height);
        for row in 0..height {
            let row_start = row * stride;
            for column in 0..width {
                let offset = row_start + column * 2;
                let low = bytes.get(offset).copied().unwrap_or(0);
                let high = bytes.get(offset + 1).copied().unwrap_or(0);
                pixels.push(u16::from(low) | (u16::from(high) << 8));
            }
        }
        pixels
    }

    /// Build one [`Frame`] from raw guest bytes / 用原始客户机字节构造一帧 [`Frame`]
    #[must_use]
    pub fn frame(&self, index: u64, steps: u64, bytes: &[u8]) -> Frame {
        Frame {
            index,
            steps,
            width: self.width,
            height: self.height,
            rgb565: self.decode(bytes),
            readable: bytes.len() >= self.buffer_len(),
        }
    }
}

/// One published frame / 已发布的一帧
///
/// `rgb565` is row-major, one `u16` per pixel, so a texture upload can hand the
/// slice straight to a GPU. Frames are immutable and shared through an
/// [`Arc`], therefore a consumer may keep one while the worker publishes the
/// next.
/// `rgb565` 按行优先、每像素一个 `u16`，可直接交给 GPU 上传纹理。帧不可变并以
/// [`Arc`] 共享，因此消费者持有一帧时 worker 仍可发布下一帧。
#[derive(Debug, Clone)]
pub struct Frame {
    /// 1-based frame counter / 从 1 开始的帧序号
    pub index: u64,
    /// Guest steps executed when the frame was captured / 抓取该帧时的客户机步数
    pub steps: u64,
    /// Pixels per row / 每行像素数
    pub width: u16,
    /// Number of rows / 行数
    pub height: u16,
    /// Row-major RGB565 pixels / 行优先 RGB565 像素
    pub rgb565: Vec<u16>,
    /// Whether the whole frame buffer could be read / 帧缓冲是否完整可读
    ///
    /// `false` means the guest window was partly unmapped; the missing pixels
    /// are black and the host should warn once instead of treating the frame as
    /// valid video.
    /// `false` 表示客户机窗口部分未映射；缺失像素为黑色，宿主应警告一次而不是把
    /// 该帧当作有效画面。
    pub readable: bool,
}

impl Frame {
    /// Whether every pixel is black / 是否所有像素都是黑色
    #[must_use]
    pub fn is_blank(&self) -> bool {
        self.rgb565.iter().all(|pixel| *pixel == 0)
    }
}

/// Parse a decimal or `0x`-prefixed hexadecimal `u32` / 解析十进制或 `0x` 十六进制 `u32`
fn parse_number(text: &str) -> Option<u32> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).ok()
    } else {
        text.parse().ok()
    }
}

/// Shared handle to one published frame / 已发布帧的共享句柄
pub type SharedFrame = Arc<Frame>;
