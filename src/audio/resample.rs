//! 重采样管线（任务 3.4）：任意采样率单声道 f32 → 16kHz i16 PCM。
//!
//! 多声道→单声道（取平均）在 worker 中完成后再进入此处；这里只负责采样率转换
//! 与 f32→i16。输入采样率已等于 16kHz 时走直通路径，避免不必要的滤波。

use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};

use super::{AudioError, TARGET_SAMPLE_RATE};

/// rubato `SincFixedIn` 每次处理的固定输入帧数。
const CHUNK: usize = 1024;

/// 单声道重采样器：直通或 sinc 重采样。
pub enum MonoResampler {
    /// 输入已是 16kHz，无需重采样。
    Passthrough,
    /// 经 rubato sinc 重采样。
    Sinc {
        resampler: SincFixedIn<f32>,
        /// 不足一个 CHUNK 的待处理输入。
        pending: Vec<f32>,
    },
}

impl MonoResampler {
    pub fn new(input_rate: u32) -> Result<Self, AudioError> {
        if input_rate == TARGET_SAMPLE_RATE {
            return Ok(Self::Passthrough);
        }

        let params = SincInterpolationParameters {
            sinc_len: 128,
            f_cutoff: 0.95,
            oversampling_factor: 128,
            interpolation: SincInterpolationType::Linear,
            window: WindowFunction::BlackmanHarris2,
        };
        let ratio = TARGET_SAMPLE_RATE as f64 / input_rate as f64;
        let resampler = SincFixedIn::<f32>::new(ratio, 1.0, params, CHUNK, 1)
            .map_err(|e| AudioError::Resampler(e.to_string()))?;

        Ok(Self::Sinc {
            resampler,
            pending: Vec::with_capacity(CHUNK * 2),
        })
    }

    /// 处理一段单声道 f32 输入，把得到的 16kHz i16 追加到 `out`。
    pub fn push(&mut self, mono: &[f32], out: &mut Vec<i16>) -> Result<(), AudioError> {
        match self {
            Self::Passthrough => out.extend(mono.iter().map(|&s| to_i16(s))),
            Self::Sinc { resampler, pending } => {
                pending.extend_from_slice(mono);
                while pending.len() >= CHUNK {
                    let chunk: Vec<f32> = pending.drain(..CHUNK).collect();
                    let result = resampler
                        .process(&[chunk], None)
                        .map_err(|e| AudioError::Resampler(e.to_string()))?;
                    out.extend(result[0].iter().map(|&s| to_i16(s)));
                }
            }
        }
        Ok(())
    }

    /// 录音结束时排空残余输入（末尾补零凑满一个 CHUNK）。
    pub fn flush(&mut self, out: &mut Vec<i16>) -> Result<(), AudioError> {
        if let Self::Sinc { resampler, pending } = self
            && !pending.is_empty()
        {
            let mut chunk = std::mem::take(pending);
            chunk.resize(CHUNK, 0.0);
            let result = resampler
                .process(&[chunk], None)
                .map_err(|e| AudioError::Resampler(e.to_string()))?;
            out.extend(result[0].iter().map(|&s| to_i16(s)));
        }
        Ok(())
    }
}

/// f32（[-1.0, 1.0]）→ i16 PCM。
fn to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
}
