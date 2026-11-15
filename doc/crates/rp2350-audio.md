# rp2350-audio — Public API / 公共 API

> Host-side audio output for the RP2350 I2S tap. [`rp2350_periph::AudioTap`]
> captures the `i16` samples a guest firmware pushes into a PIO TX FIFO; this
> crate plays them on the real sound card (optional `cpal` backend) or
> consumes them on the wall clock and discards them (null backend).
> RP2350 I2S 采样出口的宿主音频输出。[`rp2350_periph::AudioTap`] 捕获客户机固件
> 压入 PIO TX FIFO 的 `i16` 样本；本 crate 把它们播放到真实声卡（可选 `cpal`
> 后端），或按墙钟节奏消费后丢弃（null 后端）。
>
> **Status**: implemented — wall-clock worker, bounded queue, null backend,
> optional `cpal` backend with nearest-neighbour resampling.
> **状态**: 已实现 — 墙钟工作线程、有界队列、null 后端、带最近邻重采样的可选
> `cpal` 后端。

---

## 职责 / Responsibility

| Item / 项 | Description / 说明 |
|---|---|
| 采样来源 / Source | 任何实现 `SampleSource` 的对象；已为 `rp2350_periph::AudioTap` 实现 |
| 采样格式 / Format | 交替 `i16`（`AudioSink` 的 `push_word`：低半字=左，高半字=右） |
| 输出 / Output | `cpal` 声卡（feature `playback`）或 null 后端（消费即丢） |
| 线程 / Thread | 每条 `AudioOutput` 一个工作线程，`shutdown`/`Drop` 停止并 join |
| 阻塞 / Blocking | 工作线程最多睡一个 `TICK`（5 ms）；音频回调绝不等待锁以外的资源 |

`AudioOutput::start` 永不失败、永不 panic：打不开设备时静默降级到 null 后端，
模拟器继续运行。`start` 会在后端探测完成（上限 1 s）后返回，因此
`AudioOutput::backend()` 立即可用。

`AudioOutput::start` never fails and never panics: when no device can be
opened it silently degrades to the null backend and the emulator keeps running.
`start` returns after the backend probe (capped at 1 s), so
`AudioOutput::backend()` is meaningful immediately.

---

## Feature 开关 / Feature flags

```toml
[dependencies]
rp2350-audio = { workspace = true, features = ["playback"] }   # 真实声卡
rp2350-audio = { workspace = true }                            # 仅 null 后端
```

| Feature | 默认 / Default | 作用 / Effect |
|---|---|---|
| `playback` | 否 / no | 编译 `cpal` 后端（Windows WASAPI / Linux ALSA / macOS CoreAudio） |

`cpal` 是**可选依赖**，默认关闭：无设备环境与 CI 不下载、不链接任何音频库，
因此 `cargo test -p rp2350-audio` 在无声卡机器上也能全绿。
`cpal` is an **optional dependency**, off by default: device-less hosts and CI
link no audio library at all, so `cargo test -p rp2350-audio` stays green on a
machine without a sound card.

---

## 公共 API / Public API

```rust
pub const TICK: Duration;                     // 5 ms worker period / 工作线程周期

pub struct AudioFormat {
    pub rate_hz: u32,                          // sample rate in Hz / 采样率（Hz）
    pub channels: u8,                           // 1 = mono, 2 = stereo / 声道数
}
impl AudioFormat {
    pub const fn new(rate_hz: u32, channels: u8) -> Self;
    pub const fn frames_per_second(self) -> u32;    // == rate_hz
    pub const fn samples_per_second(self) -> u32;   // rate_hz * channels（饱和）
}

pub trait SampleSource: Send {
    fn take(&mut self, max: usize) -> Vec<i16>;     // oldest first / 从旧到新
    fn format(&self) -> AudioFormat;
    fn dropped(&self) -> u64;                       // source-side drops / 源丢样
}
impl SampleSource for rp2350_periph::AudioTap { /* 透传 take_samples/rate_hz/channels/dropped */ }

pub struct AudioOutput { /* private */ }
impl AudioOutput {
    pub fn start<S: SampleSource + 'static>(source: S, capacity: usize) -> Self;
    pub fn shutdown(self);                 // stop + join，毫秒级返回 / returns in ms
    pub fn playing(&self) -> bool;         // worker alive / 工作线程存活
    pub fn underruns(&self) -> u64;        // starved playback intervals / 缺样本区间
    pub fn backend(&self) -> &'static str; // "cpal" | "null"
    pub fn queued(&self) -> usize;         // internal queue depth ≤ capacity
    pub fn dropped(&self) -> u64;          // queue-side drops / 队列丢样
    pub fn capacity(&self) -> usize;
}
impl Drop for AudioOutput { /* 等价于 shutdown / same as shutdown */ }
```

---

## null 后端语义 / Null backend semantics

无 `playback` feature、无默认输出设备、声道/格式不受支持或建流失败时使用：

- 工作线程仍按 `TICK` 从 `SampleSource` 取样本并压入有界队列；
- 同一轮里按墙钟节奏（`rate_hz * channels * TICK`）从队列取走并丢弃；
- 取不满一个 `TICK` 的量时 `underruns` +1；
- 不打开任何设备、不产生宿主阻塞，`backend()` 返回 `"null"`。

When the `playback` feature is off, there is no default output device, the
channel count/sample format is unsupported, or the stream fails to build:

- the worker still pulls from `SampleSource` every `TICK` into the bounded queue;
- the same iteration drains and discards `rate_hz * channels * TICK` samples on
  the wall clock;
- a short fill increments `underruns` by one;
- no device is opened, the host never blocks, and `backend()` returns `"null"`.

---

## `cpal` 策略 / `cpal` strategy

1. **设备**：只用 `default_output_device()`；无默认输出设备即降级。
2. **配置选择**：只在支持配置中挑选与 tap **声道数相同**的范围（例如 emerald
   的 13 440 Hz 立体声只匹配 2 声道范围）。若某范围覆盖 `rate_hz`，直接使用该
   采样率（不重采样）；否则取“最近边界”离 `rate_hz` 最近的范围，实际采样率
   `clamp(rate_hz, min, max)`，并用**最近邻重采样**消除差值。
3. **样本格式**：并列时优先 `f32`，其次 `i16`；其它格式（`u16`/`i32`/`u24`…）
   视为不支持 → 降级到 null 后端。
4. **重采样**：`Resampler` 用整数累加器（`src_rate`/`dst_rate`）实现最近邻：
   升采样重复帧、降采样跳帧，不做插值，逐帧无分配。例如 tap 13 440 Hz 在
   48 000 Hz 设备上每源帧重复约 3.57 次。
5. **缺样本**：一个回调内任一顿缺样本即以静音填充该帧，并给 `underruns` +1。
6. **建流与播放**：`cpal::Stream` 在**工作线程内**创建（并非所有平台 `Send`），
   `play()` 成功才算 `cpal` 后端；任一步失败静默降级。

1. **Device**: only `default_output_device()`; no default device means degrade.
2. **Config**: only ranges whose channel count equals the tap's are considered
   (emerald's 13 440 Hz stereo only matches 2-channel ranges). A range
   containing `rate_hz` is used verbatim (no resampling); otherwise the range
   whose nearest bound is closest to `rate_hz` wins, the device rate becomes
   `clamp(rate_hz, min, max)`, and the difference is removed by
   **nearest-neighbour resampling**.
3. **Sample format**: `f32` beats `i16` on ties; other formats
   (`u16`/`i32`/`u24`…) count as unsupported → null backend.
4. **Resampling**: an integer accumulator (`src_rate`/`dst_rate`) duplicates
   frames when upsampling and skips them when downsampling; no interpolation, no
   per-frame allocation. A 13 440 Hz tap on a 48 000 Hz device repeats each
   source frame ≈3.57 times.
5. **Starvation**: any missing frame inside one callback is replaced by silence
   and increments `underruns` by one.
6. **Build/play**: the `cpal::Stream` is created **inside the worker thread**
   (`Stream` is not `Send` on every platform) and only a successful `play()`
   counts as the `cpal` backend; any failure degrades silently.

---

## 线程与容量 / Threads and capacity

```
AudioTap (guest PIO TX FIFO)
   │  take(batch)  every TICK (5 ms)
   ▼
worker thread ──push──► SampleQueue (≤ capacity samples, drop oldest + count)
   │                         │
   │ null: drain(batch)      │ cpal callback: pop frame → Resampler → device
   ▼                         ▼
discard (underruns++)      speakers (underruns++ on starvation)
```

- **有界**：队列长度 ≤ `capacity`；满时丢最旧样本并计入 `AudioOutput::dropped`。
  模拟器跑得比实时快只会丢样，不会阻塞、不会增长内存。
- **停止**：`shutdown`/`Drop` 置位 `AtomicBool`、`Condvar::notify_all` 唤醒
  工作线程并 `join`。工作线程最长睡一个 `TICK`，因此停止在毫秒级完成（验收
  预算 2 s）。
- **回调**：`cpal` 回调只持有队列锁做一次取样本 + 重采样，不做 I/O、不分配
  （scratch 缓冲复用）。
- **Bounded**: queue length ≤ `capacity`; a full queue drops the oldest samples
  and counts them in `AudioOutput::dropped`. An emulator faster than real time
  only loses samples — it never blocks and never grows memory.
- **Stop**: `shutdown`/`Drop` sets an `AtomicBool`, calls `Condvar::notify_all`
  and joins. The worker sleeps at most one `TICK`, so stopping takes
  milliseconds (acceptance budget: 2 s).
- **Callback**: the `cpal` callback only takes the queue lock, pops samples and
  resamples; no I/O, no allocation (the scratch buffer is reused).

---

## 限制 / Limitations

- 只支持与 tap 相同的声道数；单声道设备遇到立体声 tap 会降级到 null 后端。
- 只支持 `f32`/`i16` 设备格式。
- 每路 `AudioOutput` 各自消费：两个输出共享同一 tap 时样本被瓜分（不会 panic）。
- `SampleSource::take` 必须及时返回；在其中阻塞会拖慢 `shutdown` 的 join。
- 无音频时钟同步/漂移补偿：设备时钟与模拟器墙钟各自独立，长期运行可能出现
  周期性丢样或 underrun（均为静音/跳帧，不影响模拟正确性）。
- Only the tap's channel count is supported; a mono device with a stereo tap
  degrades to the null backend.
- Only `f32`/`i16` device formats are supported.
- Each `AudioOutput` consumes independently: two outputs sharing one tap split
  the samples (without panicking).
- `SampleSource::take` must return promptly; blocking there delays `shutdown`'s
  join.
- No audio-clock sync or drift compensation: the device clock and the emulator
  wall clock are independent, so long runs may show periodic drops or
  underruns (silence/skipped frames only, never a correctness issue).

---

## 使用示例 / Usage

```rust
use rp2350_audio::AudioOutput;
use rp2350_periph::{AudioSink, AudioTap};

// 客户机固件通过 PIO 压入 I2S 字；capture_capacity = tap 的样本容量。
// Guest firmware pushes I2S words through PIO; capture_capacity is the tap size.
let tap = AudioTap::new(AudioSink::new(13_440, 2, 32_768));

let output = AudioOutput::start(tap.clone(), 16_384);
println!("backend = {}", output.backend());   // "cpal" 或 "null"

// 播放期间周期性读取统计 / poll statistics while playing
let _ = output.underruns();
let _ = output.dropped();

output.shutdown();                             // 停止并 join / stop and join
```

关闭 feature 时同一段代码在无声卡机器上仍可编译、运行，只是
`backend() == "null"`。
With the feature disabled the same code still builds and runs on a device-less
machine, except `backend() == "null"`.
