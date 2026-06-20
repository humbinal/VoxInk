//! 应用内 WAV 回放 —— 片段试听（IDEAS：片段播放改为应用内实现）。
//!
//! 基于 rodio（纯 Rust，底层走 cpal/WASAPI，无 C 依赖）。WAV 由 hound 解码为 i16 后
//! 以 [`SamplesBuffer`] 喂入，rodio 自动完成与输出设备的重采样/声道映射。
//!
//! ⚠️ [`OutputStream`] 是 `!Send` 且 **drop 即停**，必须随播放存活（存入视图字段）。

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use rodio::buffer::SamplesBuffer;
use rodio::{OutputStream, OutputStreamHandle, Sink};

/// 单文件 WAV 回放器：持有输出流 + Sink，提供暂停/继续/进度查询。
pub struct Player {
    /// 输出流：drop 即停，仅需保活，不直接使用。
    _stream: OutputStream,
    /// 输出流句柄：仅需保活。
    _handle: OutputStreamHandle,
    sink: Sink,
    /// 当前曲目总时长（由样本数推算）。
    total: Duration,
}

impl Player {
    /// 解码并立即开始播放一个 WAV 文件。
    pub fn play_wav(path: &Path) -> Result<Self> {
        let (stream, handle) = OutputStream::try_default().context("打开音频输出设备失败")?;
        let sink = Sink::try_new(&handle).context("创建播放 Sink 失败")?;

        let (samples, channels, sample_rate) = decode_wav_i16(path)?;
        let frames = (samples.len() / channels.max(1) as usize) as f64;
        let total = Duration::from_secs_f64(frames / sample_rate.max(1) as f64);

        sink.append(SamplesBuffer::new(channels, sample_rate, samples));
        Ok(Self {
            _stream: stream,
            _handle: handle,
            sink,
            total,
        })
    }

    /// 在「暂停」与「继续」之间切换。
    pub fn toggle_pause(&self) {
        if self.sink.is_paused() {
            self.sink.play();
        } else {
            self.sink.pause();
        }
    }

    /// 是否处于暂停态。
    pub fn is_paused(&self) -> bool {
        self.sink.is_paused()
    }

    /// 是否播放完毕（队列已空）。
    pub fn is_finished(&self) -> bool {
        self.sink.empty()
    }

    /// 当前播放进度 0.0..=1.0（用于绘制条目背景进度条）。
    pub fn progress(&self) -> f32 {
        let total = self.total.as_secs_f32();
        if total <= 0.0 {
            return 0.0;
        }
        (self.sink.get_pos().as_secs_f32() / total).clamp(0.0, 1.0)
    }
}

/// 解码 WAV 为交错 i16 样本，返回 `(样本, 声道数, 采样率)`。
///
/// 本应用录音固定为 16-bit Int 单声道 16kHz；此处仍对 24/32-bit Int 与 Float 做容错，
/// 以便用户导入/旧文件也能试听。
fn decode_wav_i16(path: &Path) -> Result<(Vec<i16>, u16, u32)> {
    let mut reader = hound::WavReader::open(path).context("打开 WAV 文件失败")?;
    let spec = reader.spec();
    let samples: Vec<i16> = match spec.sample_format {
        hound::SampleFormat::Int => match spec.bits_per_sample {
            16 => reader
                .samples::<i16>()
                .collect::<std::result::Result<_, _>>()?,
            bits => {
                let shift = bits.saturating_sub(16);
                reader
                    .samples::<i32>()
                    .map(|s| s.map(|v| (v >> shift) as i16))
                    .collect::<std::result::Result<_, _>>()?
            }
        },
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .map(|s| s.map(|v| (v.clamp(-1.0, 1.0) * i16::MAX as f32) as i16))
            .collect::<std::result::Result<_, _>>()?,
    };
    Ok((samples, spec.channels, spec.sample_rate))
}
