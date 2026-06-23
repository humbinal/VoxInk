//! 外部音频导入的时长探测（M14，§4.2.3）。
//!
//! wav 用 hound 读 header；mp3 用 symphonia（纯 Rust）demux 累计时长——只读包、不解码 PCM，
//! 故对几分钟的语音文件开销可忽略。两者均返回秒（向下取整），失败回 [`AudioError::Decode`]。

use std::path::Path;

use crate::asr::traits::AudioFormat;

use super::AudioError;

/// 探测音频文件时长（秒，向下取整）。仅支持 wav/mp3；其它扩展名返回错误。
pub fn audio_duration_secs(path: &Path) -> Result<u32, AudioError> {
    match AudioFormat::from_path(path) {
        Some(AudioFormat::Wav) => wav_duration_secs(path),
        Some(AudioFormat::Mp3) => mp3_duration_secs(path),
        None => Err(AudioError::Decode(format!(
            "不支持的音频格式: {}",
            path.display()
        ))),
    }
}

/// WAV：头部直接给出采样率与每声道样本数，无需解码。
fn wav_duration_secs(path: &Path) -> Result<u32, AudioError> {
    let reader = hound::WavReader::open(path).map_err(|e| AudioError::Decode(e.to_string()))?;
    let sr = reader.spec().sample_rate;
    if sr == 0 {
        return Ok(0);
    }
    // `duration()` = 每声道样本数。
    Ok(reader.duration() / sr)
}

/// MP3：优先用容器声明的总帧数（含 Xing/Info 头时存在），否则 demux 累计各包时长。
fn mp3_duration_secs(path: &Path) -> Result<u32, AudioError> {
    use symphonia::core::errors::Error as SymError;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let file = std::fs::File::open(path).map_err(|e| AudioError::Decode(e.to_string()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("mp3");
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| AudioError::Decode(e.to_string()))?;
    let mut format = probed.format;

    // 复制出 Copy 的元参数，释放对 `format` 的借用，以便后续 `next_packet`（需 &mut）。
    let (time_base, n_frames, sample_rate) = {
        let track = format
            .default_track()
            .ok_or_else(|| AudioError::Decode("mp3 无音轨".to_string()))?;
        (
            track.codec_params.time_base,
            track.codec_params.n_frames,
            track.codec_params.sample_rate,
        )
    };

    // 容器已声明总帧数：直接换算。
    if let (Some(tb), Some(n)) = (time_base, n_frames) {
        return Ok(tb.calc_time(n).seconds as u32);
    }

    // 否则累计各包时长（按 time_base 单位；只读包不解码）。
    let mut total: u64 = 0;
    loop {
        match format.next_packet() {
            Ok(packet) => total += packet.dur,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymError::ResetRequired) => break,
            Err(e) => return Err(AudioError::Decode(e.to_string())),
        }
    }
    let secs = match time_base {
        Some(tb) => tb.calc_time(total).seconds,
        None => match sample_rate {
            Some(sr) if sr > 0 => total / sr as u64,
            _ => 0,
        },
    };
    Ok(secs as u32)
}
