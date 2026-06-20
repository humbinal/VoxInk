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
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

pub use capture::{Recorder, list_input_devices};
pub use chunk_sender::StreamingCapture;

/// ASR 统一要求的目标采样率（§4.1）。
pub const TARGET_SAMPLE_RATE: u32 = 16_000;

/// 实时电平表：录音 worker 写入最近音频的电平包络幅度（0..1，经 [`LevelEnvelope`] 平滑，
/// 以 f32 位存于原子），UI 轮询读取绘制波形。
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

/// 电平包络跟随器：**瞬时起、缓慢落**（峰值保持）。
///
/// worker 每收到一段样本就 [`push`](Self::push) 一次：响度变大时立刻跳到新值，
/// 变小时按 [`Self::RELEASE`] 缓慢回落。这样能"抓住"采集块之间的瞬时峰值
/// （UI 每 ~60ms 才轮询一次，纯瞬时 RMS 会漏掉两次轮询间的响度起伏），
/// 让电平表对音量变化的感知更敏感、更"跟手"。
#[derive(Debug, Default)]
pub struct LevelEnvelope {
    value: f32,
}

impl LevelEnvelope {
    /// 无新峰值时每次 push 衰减到上次的比例（越接近 1 落得越慢、保持越久）。
    const RELEASE: f32 = 0.80;

    pub fn new() -> Self {
        Self::default()
    }

    /// 输入一段样本，更新包络并返回当前幅度（0..1）。
    pub fn push(&mut self, samples: &[f32]) -> f32 {
        let rms = rms_amplitude(samples);
        self.value = if rms >= self.value {
            rms
        } else {
            self.value * Self::RELEASE
        };
        self.value
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_attacks_instantly_and_releases_slowly() {
        let mut env = LevelEnvelope::new();
        // 瞬时起：一段大响度立刻把包络抬到该 RMS。
        let loud = [0.5_f32; 64];
        let rms = rms_amplitude(&loud);
        assert!((env.push(&loud) - rms).abs() < 1e-6);
        // 缓慢落：随后静音段不会立刻归零，而是按 RELEASE 比例回落。
        let silent = [0.0_f32; 64];
        let after = env.push(&silent);
        assert!(after < rms && after > rms * 0.5, "应缓慢回落: {after}");
        assert!((after - rms * LevelEnvelope::RELEASE).abs() < 1e-6);
    }

    #[test]
    fn envelope_tracks_rising_loudness() {
        let mut env = LevelEnvelope::new();
        let a = env.push(&[0.1_f32; 64]);
        let b = env.push(&[0.4_f32; 64]);
        assert!(b > a, "响度上升包络应随之上升");
    }
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
