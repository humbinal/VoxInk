//! WAV 文件写入（任务 3.5）：hound 流式写入 16kHz/16-bit/mono PCM。

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use chrono::Local;
use hound::{SampleFormat, WavSpec, WavWriter};

use super::{AudioError, TARGET_SAMPLE_RATE};

/// 目标 WAV 写入器类型。
pub type WavSink = WavWriter<BufWriter<File>>;

/// 临时 WAV 路径：`{临时目录}/voxink_recording_{YYYYMMDD}_{HHMMSS}.wav`（§4.2.2）。
pub fn temp_wav_path() -> PathBuf {
    let ts = Local::now().format("%Y%m%d_%H%M%S");
    std::env::temp_dir().join(format!("voxink_recording_{ts}.wav"))
}

/// 创建 16kHz/16-bit/mono PCM 的 WAV 写入器。
pub fn create_writer(path: &Path) -> Result<WavSink, AudioError> {
    let spec = WavSpec {
        channels: 1,
        sample_rate: TARGET_SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    WavWriter::create(path, spec).map_err(|e| AudioError::Wav(e.to_string()))
}
