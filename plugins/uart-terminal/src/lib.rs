//! UART terminal device plugin — decodes a bit-banged UART line into bytes
//! UART 终端设备插件 — 将位时序 UART 线路解码为字节
//!
//! The plugin exposes two ports: `TX` (input — the plugin samples the level
//! the MCU drives, host → device direction) and `RX` (output — the plugin
//! drives the MCU pin, device → host direction). Bytes received on `TX` are
//! decoded from start bit + data bits (LSB first) + stop bit sampling, and
//! [`UartTerminalPlugin::send_byte`] queues bytes that are shifted out on
//! `RX` using the same bit timing.
//! 插件暴露两个端口：`TX`（输入 —— 插件采样 MCU 驱动的电平，host → device
//! 方向）与 `RX`（输出 —— 插件驱动 MCU 引脚，device → host 方向）。`TX` 上
//! 收到的字节按 起始位 + 数据位（LSB 优先）+ 停止位 采样解码；
//! [`UartTerminalPlugin::send_byte`] 将字节排入队列，并以相同位时序在 `RX`
//! 上移出。
//!
//! # Timing model / 时序模型
//!
//! [`rp2350_plugin::PluginContext::current_cycles`] only reports cycles, so the
//! plugin converts them with the configured `clock_hz` (default 150 MHz, the
//! RP2350 default system clock):
//! `cycles_per_bit = clock_hz / baud`. Tests may set `clock_hz = 1_000_000`
//! to treat one cycle as one microsecond.
//! [`rp2350_plugin::PluginContext::current_cycles`] 只提供周期数，因此插件用
//! 配置项 `clock_hz`（默认 150 MHz，RP2350 默认系统时钟）换算：
//! `cycles_per_bit = clock_hz / baud`。测试可设 `clock_hz = 1_000_000`，
//! 使 1 周期等于 1 微秒。
//!
//! # Limitations / 限制
//!
//! There is no MCP entry point for pushing bytes into a plugin yet, so the
//! device → host (`RX`) direction is only reachable from library callers and
//! tests (`send_byte` / `send_bytes`); wiring it into MCP is pending.
//! 目前没有从 MCP 向插件写入字节的入口，因此 device → host（`RX`）方向只能
//! 通过库调用与测试（`send_byte` / `send_bytes`）触发，MCP 接线待完成。

// 插件 ABI 通过 trait 对象指针跨越动态库边界，依赖同一编译器与 ABI 版本。
// The plugin ABI passes trait-object pointers across the dynamic library
// boundary, relying on the same compiler and ABI version.
#![allow(improper_ctypes_definitions)]
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]
// UART 收发天然成对命名（rx/tx），位采样模型需要整数缩放与位掩码；
// 这些是串口逐位仿真的固有特性。
// UART naturally uses paired rx/tx names, and bit-sampling needs integer
// scaling and masks; both are inherent to serial bit-level emulation.
#![allow(
    clippy::similar_names,
    clippy::cast_possible_truncation,
    clippy::cast_lossless
)]

use std::collections::VecDeque;

use rp2350_plugin::abi::ABI_VERSION;
use rp2350_plugin::{PluginContext, PluginError, PluginMetadata, RpPlugin};
use rp2350_virt_periph::ConfigSchema;
use rp2350_virt_periph::device::{DevicePlugin, DevicePort};
use rp2350_virt_periph::error::ConfigError;

/// Stable plugin identifier / 稳定的插件标识符
const PLUGIN_ID: &str = "com.rp2350-rs.uart-terminal";

/// Default line speed in bits per second / 默认波特率（bit/s）
const DEFAULT_BAUD: u32 = 115_200;

/// Default simulated clock feeding `PluginContext::current_cycles`
/// 默认驱动 `PluginContext::current_cycles` 的仿真时钟
const DEFAULT_CLOCK_HZ: u64 = 150_000_000;

/// Default number of data bits per frame / 每帧默认数据位数
const DEFAULT_DATA_BITS: u8 = 8;

/// Default number of stop bits per frame / 每帧默认停止位数
const DEFAULT_STOP_BITS: u8 = 1;

/// Upper bound of the retained receive buffer / 接收缓冲区保留上限
const MAX_RX_BUFFER: usize = 4096;

/// Number of trailing bytes reported in the snapshot
/// 状态快照中报告的尾部字节数
const RECENT_BYTES: usize = 16;

/// Highest accepted baud rate / 允许的最高波特率
const MAX_BAUD: u64 = 10_000_000;

/// Sampling state of one incoming frame / 单个接收帧的采样状态
#[derive(Debug, Clone, Copy)]
struct RxFrame {
    /// Cycle at which the start edge was observed / 检测到起始边沿的周期
    start_cycle: u64,
    /// Index of the bit being sampled next (0 = first data bit)
    /// 下一个待采样位的下标（0 = 第一个数据位）
    bit: u8,
    /// Accumulated data bits, LSB first / 累积的数据位（LSB 优先）
    value: u8,
}

/// One outgoing frame shifted out on the `RX` port / 从 `RX` 端口移出的发送帧
#[derive(Debug, Clone, Copy)]
struct TxFrame {
    /// Byte being shifted out / 正在移出的字节
    byte: u8,
    /// Cycle at which the start bit began / 起始位开始的周期
    start_cycle: u64,
}

/// Configurable UART terminal device (bit-level UART decoder + serializer)
/// 可配置的 UART 终端设备（位级 UART 解码器 + 串行化器）
pub struct UartTerminalPlugin {
    /// Plugin metadata returned to the host / 返回给主机的插件元数据
    metadata: PluginMetadata,
    /// Exposed ports: `[TX (input), RX (output)]` / 暴露端口
    ports: Vec<DevicePort>,
    /// Clock frequency used to convert cycles to bit times
    /// 用于把周期换算为位时间的时钟频率
    clock_hz: u64,
    /// Line speed in bits per second / 波特率
    baud: u32,
    /// Data bits per frame (5..=8) / 每帧数据位数
    data_bits: u8,
    /// Stop bits per frame (1..=2) / 每帧停止位数
    stop_bits: u8,
    /// Idle level of the `RX` line (config field `rx_line`)
    /// `RX` 线路空闲电平（配置项 `rx_line`）
    rx_line: bool,
    /// Latest level sampled on the `TX` port / `TX` 端口最近采样电平
    tx_level: bool,
    /// In-flight receive frame, if any / 正在接收的帧
    rx_frame: Option<RxFrame>,
    /// Set once the line has been idle since the previous frame
    /// 上一帧之后线路上出现过空闲电平
    rx_saw_idle: bool,
    /// Total bytes decoded from `TX` / 从 `TX` 解码的总字节数
    rx_total: u64,
    /// Last decoded byte / 最近解码的字节
    last_rx: Option<u8>,
    /// Ring of recently decoded bytes / 近期解码字节的缓冲
    rx_buffer: Vec<u8>,
    /// Frames discarded for a bad stop bit or timeout / 因停止位错误或超时丢弃的帧数
    framing_errors: u64,
    /// In-flight outgoing frame, if any / 正在发送的帧
    tx_frame: Option<TxFrame>,
    /// Bytes waiting to be shifted out on `RX` / 等待在 `RX` 上移出的字节
    tx_queue: VecDeque<u8>,
    /// Total bytes shifted out on `RX` / 在 `RX` 上移出的总字节数
    tx_total: u64,
    /// Last byte shifted out on `RX` / 最近移出的字节
    last_tx: Option<u8>,
}

impl UartTerminalPlugin {
    /// Create a plugin with the default line configuration (115200 8N1)
    /// 创建使用默认线路配置（115200 8N1）的插件
    #[must_use]
    pub fn new() -> Self {
        let idle = true;
        let mut ports = vec![
            DevicePort::input("TX", "MCU TX sampled by the terminal / 终端采样的 MCU TX"),
            DevicePort::output("RX", "Terminal RX driving the MCU / 终端驱动 MCU 的 RX"),
        ];
        // TX 线路空闲为高，RX 输出同样从空闲电平起步。
        // The TX line idles high; the RX output starts at the idle level too.
        ports[0].level = idle;
        ports[1].level = idle;
        Self {
            metadata: PluginMetadata::builder()
                .id(PLUGIN_ID)
                .name("UART_TERMINAL")
                .version("0.1.0")
                .description("Bit-level UART terminal (115200 8N1) / 位级 UART 终端")
                .author("RP2350-rs Team")
                .abi_version(ABI_VERSION)
                .min_host_version("0.1.0")
                .has_ui(false)
                .tags(vec!["uart".to_string(), "terminal".to_string()])
                .build(),
            ports,
            clock_hz: DEFAULT_CLOCK_HZ,
            baud: DEFAULT_BAUD,
            data_bits: DEFAULT_DATA_BITS,
            stop_bits: DEFAULT_STOP_BITS,
            rx_line: idle,
            tx_level: idle,
            rx_frame: None,
            rx_saw_idle: true,
            rx_total: 0,
            last_rx: None,
            rx_buffer: Vec::new(),
            framing_errors: 0,
            tx_frame: None,
            tx_queue: VecDeque::new(),
            tx_total: 0,
            last_tx: None,
        }
    }

    /// Current baud rate / 当前波特率
    #[must_use]
    pub fn baud(&self) -> u32 {
        self.baud
    }

    /// Current clock frequency used for cycle conversion
    /// 当前用于周期换算的时钟频率
    #[must_use]
    pub fn clock_hz(&self) -> u64 {
        self.clock_hz
    }

    /// Cycles per bit derived from `clock_hz / baud` (never zero)
    /// 由 `clock_hz / baud` 推出的每比特周期数（不会为 0）
    #[must_use]
    pub fn bit_cycles(&self) -> u64 {
        (self.clock_hz / u64::from(self.baud)).max(1)
    }

    /// Bytes decoded so far (bounded by the 4096-byte rolling buffer)
    /// 目前已解码的字节（受 4096 字节滚动缓冲限制）
    #[must_use]
    pub fn rx_buffer(&self) -> &[u8] {
        &self.rx_buffer
    }

    /// Last byte decoded from `TX`, if any / 最近从 `TX` 解码的字节
    #[must_use]
    pub fn last_rx_byte(&self) -> Option<u8> {
        self.last_rx
    }

    /// Queue one byte to be shifted out on `RX` (device → host)
    /// 将一个字节排入队列，稍后在 `RX` 上移出（device → host）
    ///
    /// The byte leaves on the next [`RpPlugin::tick`] calls at `baud` timing.
    /// 该字节将在后续 [`RpPlugin::tick`] 调用中按 `baud` 时序移出。
    pub fn send_byte(&mut self, byte: u8) {
        self.tx_queue.push_back(byte);
    }

    /// Queue several bytes to be shifted out on `RX`, in order
    /// 按顺序将多个字节排入 `RX` 发送队列
    pub fn send_bytes(&mut self, bytes: &[u8]) {
        self.tx_queue.extend(bytes);
    }

    /// Number of bytes still waiting to be shifted out
    /// 仍在等待移出的字节数
    #[must_use]
    pub fn pending_tx(&self) -> usize {
        self.tx_queue.len() + usize::from(self.tx_frame.is_some())
    }

    /// Idle (mark) level of the line / 线路空闲（mark）电平
    fn idle_level(&self) -> bool {
        self.rx_line
    }

    /// Store a decoded byte and update the rolling buffer
    /// 保存解码字节并更新滚动缓冲
    fn accept_byte(&mut self, byte: u8) {
        self.rx_total += 1;
        self.last_rx = Some(byte);
        self.rx_buffer.push(byte);
        if self.rx_buffer.len() > MAX_RX_BUFFER {
            let excess = self.rx_buffer.len() - MAX_RX_BUFFER;
            self.rx_buffer.drain(..excess);
        }
    }

    /// Sample the `TX` line at UART bit centers / 在 UART 位中心采样 `TX` 线路
    fn poll_receive(&mut self, now: u64) {
        let bit = self.bit_cycles();
        let idle = self.idle_level();
        let frame_span =
            bit.saturating_mul(u64::from(self.data_bits) + u64::from(self.stop_bits) + 1);

        if self.rx_frame.is_none() {
            if self.tx_level == idle {
                self.rx_saw_idle = true;
                return;
            }
            // 未观察到空闲电平前的低电平不视为起始位（避免帧尾重同步）。
            // A low level before an observed idle period is not a start bit.
            if !self.rx_saw_idle {
                return;
            }
            self.rx_frame = Some(RxFrame {
                start_cycle: now,
                bit: 0,
                value: 0,
            });
        }

        // 一次 tick 可能跨过多个采样点（主机按大块周期推进）。
        // A single tick may skip several sample points if the host advances
        // in large chunks.
        while let Some(mut frame) = self.rx_frame.take() {
            if now >= frame.start_cycle.saturating_add(frame_span) && frame.bit < self.data_bits {
                // 超时：线路未按位时序保持，丢弃该帧。
                // Timeout: the line did not hold the expected bit timing.
                self.framing_errors += 1;
                self.rx_saw_idle = false;
                break;
            }
            // 采样点位于每位中心：1.5 位起、步长 1 位。
            // Sample points sit at bit centers: start at 1.5 bits, step 1 bit.
            let offset = bit
                .saturating_mul(u64::from(frame.bit) * 2 + 3)
                .saturating_div(2);
            if now < frame.start_cycle.saturating_add(offset) {
                self.rx_frame = Some(frame);
                break;
            }
            let level = self.tx_level;
            if frame.bit < self.data_bits {
                if level {
                    frame.value |= 1u8 << frame.bit;
                }
                frame.bit += 1;
                self.rx_frame = Some(frame);
                continue;
            }
            if level == idle {
                self.accept_byte(frame.value);
            } else {
                self.framing_errors += 1;
            }
            self.rx_saw_idle = false;
            break;
        }
    }

    /// Drive the `RX` output for the queued bytes / 驱动 `RX` 输出移出排队字节
    fn poll_transmit(&mut self, now: u64) {
        let bit = self.bit_cycles();
        let idle = self.idle_level();
        let total = self.data_bits + self.stop_bits + 1;

        loop {
            if self.tx_frame.is_none() {
                let Some(byte) = self.tx_queue.pop_front() else {
                    self.ports[1].level = idle;
                    return;
                };
                self.tx_frame = Some(TxFrame {
                    byte,
                    start_cycle: now,
                });
            }
            let Some(frame) = self.tx_frame else {
                return;
            };
            let elapsed = now.saturating_sub(frame.start_cycle) / bit;
            let index = u8::try_from(elapsed).unwrap_or(u8::MAX).min(total);
            if index >= total {
                self.ports[1].level = idle;
                self.tx_total += 1;
                self.last_tx = Some(frame.byte);
                self.tx_frame = None;
                continue;
            }
            self.ports[1].level = match index {
                0 => !idle,
                i if i <= self.data_bits => (frame.byte >> (i - 1)) & 1 == 1,
                _ => idle,
            };
            return;
        }
    }

    /// Snapshot of the trailing bytes as lowercase hex / 尾部字节的小写十六进制
    fn recent_hex(&self) -> String {
        hex_string(&self.recent_bytes())
    }

    /// Snapshot of the trailing bytes as printable ASCII / 尾部字节的可打印 ASCII
    fn recent_ascii(&self) -> String {
        ascii_string(&self.recent_bytes())
    }

    /// Copy of the last 16 decoded bytes / 最近 16 个解码字节的副本
    fn recent_bytes(&self) -> Vec<u8> {
        let start = self.rx_buffer.len().saturating_sub(RECENT_BYTES);
        self.rx_buffer[start..].to_vec()
    }
}

impl Default for UartTerminalPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl RpPlugin for UartTerminalPlugin {
    fn metadata(&self) -> &PluginMetadata {
        &self.metadata
    }

    fn init(&mut self, ctx: &mut dyn PluginContext) -> Result<(), PluginError> {
        ctx.log_info("UART terminal initialized / UART 终端已初始化");
        Ok(())
    }

    fn tick(&mut self, ctx: &mut dyn PluginContext, _cycles: u64) {
        // 采用绝对周期而非增量，避免主机分块推进时累计误差。
        // Use absolute cycles instead of deltas so chunked host progress
        // cannot accumulate drift.
        let now = ctx.current_cycles();
        self.poll_receive(now);
        self.poll_transmit(now);
    }

    fn shutdown(&mut self) {
        self.rx_frame = None;
        self.tx_frame = None;
        self.tx_queue.clear();
        self.tx_level = self.idle_level();
        self.rx_saw_idle = true;
        let idle = self.rx_line;
        if let Some(tx_port) = self.ports.get_mut(0) {
            tx_port.level = idle;
        }
        if let Some(rx_port) = self.ports.get_mut(1) {
            rx_port.level = idle;
        }
    }

    fn state_snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "name": self.metadata.name,
            "baud": self.baud,
            "data_bits": self.data_bits,
            "stop_bits": self.stop_bits,
            "rx_line": self.rx_line,
            "clock_hz": self.clock_hz,
            "bit_cycles": self.bit_cycles(),
            "rx_bytes": self.rx_total,
            "tx_bytes": self.tx_total,
            "last_byte": self.last_rx,
            "last_tx_byte": self.last_tx,
            "buffer_len": self.rx_buffer.len(),
            "framing_errors": self.framing_errors,
            "recent_hex": self.recent_hex(),
            "recent_ascii": self.recent_ascii(),
        })
    }
}

impl DevicePlugin for UartTerminalPlugin {
    fn ports(&self) -> &[DevicePort] {
        &self.ports
    }

    fn ports_mut(&mut self) -> &mut [DevicePort] {
        &mut self.ports
    }

    fn on_gpio_change(&mut self, port_index: usize, level: bool) {
        // 端口 0 是 TX（输入）；端口 1（RX，输出）由插件驱动，忽略外部写入。
        // Port 0 is TX (input); port 1 (RX, output) is driven by the plugin.
        if port_index != 0 {
            return;
        }
        self.tx_level = level;
        self.ports[0].level = level;
    }

    fn config_schema(&self) -> Option<ConfigSchema> {
        Some(
            ConfigSchema::new("UART Terminal", "UART line settings / UART 线路设置")
                .with_property(
                    "baud",
                    serde_json::json!({
                        "type": "integer",
                        "default": DEFAULT_BAUD,
                        "minimum": 1,
                        "maximum": MAX_BAUD,
                    }),
                    false,
                )
                .with_property(
                    "data_bits",
                    serde_json::json!({ "type": "integer", "enum": [5, 6, 7, 8], "default": 8 }),
                    false,
                )
                .with_property(
                    "stop_bits",
                    serde_json::json!({ "type": "integer", "enum": [1, 2], "default": 1 }),
                    false,
                )
                .with_property(
                    "rx_line",
                    serde_json::json!({ "type": "boolean", "default": true }),
                    false,
                )
                .with_property(
                    "clock_hz",
                    serde_json::json!({
                        "type": "integer",
                        "default": DEFAULT_CLOCK_HZ,
                        "minimum": 1,
                    }),
                    false,
                ),
        )
    }

    fn apply_config(&mut self, config: &serde_json::Value) -> Result<(), ConfigError> {
        let clock_hz = read_u64(config, "clock_hz")?.unwrap_or(self.clock_hz);
        let baud = read_u64(config, "baud")?.unwrap_or(u64::from(self.baud));
        let data_bits = read_u64(config, "data_bits")?.unwrap_or(u64::from(self.data_bits));
        let stop_bits = read_u64(config, "stop_bits")?.unwrap_or(u64::from(self.stop_bits));
        let rx_line = read_bool(config, "rx_line")?.unwrap_or(self.rx_line);

        if clock_hz == 0 {
            return Err(out_of_range("clock_hz", "must be > 0 / 必须大于 0"));
        }
        if baud == 0 || baud > MAX_BAUD {
            return Err(out_of_range("baud", "must be 1..=10000000"));
        }
        if baud > clock_hz {
            return Err(out_of_range(
                "baud",
                "must not exceed clock_hz / 不能超过 clock_hz",
            ));
        }
        if !(5..=8).contains(&data_bits) {
            return Err(out_of_range("data_bits", "must be 5..=8"));
        }
        if !(1..=2).contains(&stop_bits) {
            return Err(out_of_range("stop_bits", "must be 1..=2"));
        }

        self.clock_hz = clock_hz;
        self.baud = u32::try_from(baud).unwrap_or(DEFAULT_BAUD);
        self.data_bits = u8::try_from(data_bits).unwrap_or(DEFAULT_DATA_BITS);
        self.stop_bits = u8::try_from(stop_bits).unwrap_or(DEFAULT_STOP_BITS);
        self.rx_line = rx_line;
        // 配置变更后丢弃半帧，避免用新时序解释旧帧。
        // Drop any half frame so stale bits are not decoded with new timing.
        self.rx_frame = None;
        self.rx_saw_idle = self.tx_level == rx_line;
        if self.tx_frame.is_none() {
            self.ports[1].level = rx_line;
        }
        Ok(())
    }
}

/// Read an optional unsigned integer field / 读取可选的无符号整数字段
fn read_u64(config: &serde_json::Value, field: &str) -> Result<Option<u64>, ConfigError> {
    match config.get(field) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| ConfigError::InvalidType {
                field: field.to_string(),
                expected: "non-negative integer / 非负整数".to_string(),
                actual: value.to_string(),
            }),
    }
}

/// Read an optional boolean field / 读取可选的布尔字段
fn read_bool(config: &serde_json::Value, field: &str) -> Result<Option<bool>, ConfigError> {
    match config.get(field) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => value
            .as_bool()
            .map(Some)
            .ok_or_else(|| ConfigError::InvalidType {
                field: field.to_string(),
                expected: "boolean / 布尔值".to_string(),
                actual: value.to_string(),
            }),
    }
}

/// Build an out-of-range configuration error / 构造越界配置错误
fn out_of_range(field: &str, message: &str) -> ConfigError {
    ConfigError::OutOfRange {
        field: field.to_string(),
        message: message.to_string(),
    }
}

/// Render bytes as space-separated lowercase hex / 以空格分隔的小写十六进制渲染
fn hex_string(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Render bytes as printable ASCII, non-printables as `.`
/// 渲染为可打印 ASCII，不可打印字符显示为 `.`
fn ascii_string(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&byte| {
            if (0x20..=0x7e).contains(&byte) {
                char::from(byte)
            } else {
                '.'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;

/// Return the ABI version this plugin was compiled against
/// 返回此插件编译时使用的 ABI 版本
#[unsafe(no_mangle)]
pub extern "C" fn rp2350_plugin_version() -> u32 {
    ABI_VERSION
}

/// Report that this library exports a device plugin
/// 报告此库导出的是设备插件
#[unsafe(no_mangle)]
pub extern "C" fn rp2350_plugin_is_device() -> u32 {
    1
}

/// Create a boxed UART terminal device plugin for the host
/// 为主机创建 boxed UART 终端设备插件
#[unsafe(no_mangle)]
pub extern "C" fn rp2350_device_plugin_create() -> *mut dyn DevicePlugin {
    Box::into_raw(Box::new(UartTerminalPlugin::new()))
}
