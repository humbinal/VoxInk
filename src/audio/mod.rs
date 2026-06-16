//! 本地录音引擎 —— M3（§3.2 数据流 / §3.3 线程模型）。
//!
//! 管线：麦克风 →[cpal 回调]→ 环形缓冲区(f32) →[worker 线程]→ 降混单声道 →
//! rubato 重采样至 16kHz → f32→i16 → hound 流式写入 WAV。
//!
//! ⚠️ 关键约束（§3.3）：音频回调仅做"格式转换 + 写入环形缓冲区"，绝不阻塞；
//! 重采样、编码、文件 I/O 全部在 worker 线程完成。

pub mod buffer;
pub mod capture;
pub mod chunk_sender;
pub mod resample;
pub mod writer;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub use capture::Recorder;
pub use chunk_sender::StreamingCapture;

/// ASR 统一要求的目标采样率（§4.1）。
pub const TARGET_SAMPLE_RATE: u32 = 16_000;

/// 实时电平表：录音 worker 写入最近一段音频的峰值幅度（0..1，以 f32 位存于原子），UI 轮询读取绘制波形。
pub type LevelMeter = Arc<AtomicU32>;

/// 写入当前电平（峰值幅度 0..1）。
pub fn store_level(meter: &AtomicU32, value: f32) {
    meter.store(value.to_bits(), Ordering::Relaxed);
}

/// 读取当前电平（峰值幅度 0..1）。
pub fn load_level(meter: &AtomicU32) -> f32 {
    f32::from_bits(meter.load(Ordering::Relaxed))
}

/// 计算一段 f32 样本的 RMS（均方根）幅度，反映感知响度/能量。
///
/// 不用峰值：语音瞬态尖峰极易逼近满量程，峰值表会"动不动就满格"；RMS 比峰值低约 15-20dB
/// 且更平滑，更适合做电平表（§6.2 波形）。
pub fn rms_amplitude(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|&s| s * s).sum();
    (sum_sq / samples.len() as f32).sqrt()
}

/// 音频子系统错误（任务 3.1：设备不可用返回明确错误类型）。
#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("未检测到可用的麦克风设备")]
    NoInputDevice,

    #[error("获取默认输入配置失败: {0}")]
    DefaultConfig(String),

    #[error("不支持的采样格式: {0}")]
    UnsupportedFormat(String),

    #[error("构建音频输入流失败: {0}")]
    BuildStream(String),

    #[error("启动音频流失败: {0}")]
    PlayStream(String),

    #[error("初始化重采样器失败: {0}")]
    Resampler(String),

    #[error("WAV 写入失败: {0}")]
    Wav(String),

    #[error("录音线程异常退出")]
    WorkerPanicked,
}

/// 一次录音的产物。
#[derive(Debug, Clone)]
pub struct RecordingOutcome {
    /// 生成的 WAV 文件路径（16kHz/16-bit/mono PCM）。
    pub path: PathBuf,
    /// 写入的 16kHz 单声道样本数。
    pub frames: u64,
    /// 录音时长。
    pub duration: Duration,
}
