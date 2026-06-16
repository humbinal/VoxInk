//! 流式音频管道（M6，任务 6.3）。
//!
//! 采集 → 降混 → 重采样 16kHz i16 → 100ms 帧 → MPSC（送流式后端）；
//! **同时写本地 WAV**，供实时识别失败时回退离线转写（数据不丢失，§4.2.1）。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::StreamTrait;
use ringbuf::traits::Consumer;
use tokio::sync::mpsc::Sender;

use super::buffer::AudioCons;
use super::capture::open_capture;
use super::resample::MonoResampler;
use super::writer::{create_writer, WavSink};
use super::{AudioError, LevelMeter, RecordingOutcome};

/// 16kHz 下 100ms = 1600 样本；i16 即 3200 字节。
const FRAME_SAMPLES: usize = 1600;
const FRAME_BYTES: usize = FRAME_SAMPLES * 2;

/// 流式采集会话。持有 cpal 流（主线程）、停止标志与 worker 句柄。
/// `stop()` 返回的 `RecordingOutcome` 含完整 WAV 路径，供失败回退离线转写。
pub struct StreamingCapture {
    stream: cpal::Stream,
    stop_flag: Arc<AtomicBool>,
    worker: Option<JoinHandle<Result<RecordingOutcome, AudioError>>>,
}

impl StreamingCapture {
    /// 启动流式采集：PCM 100ms 帧经 `audio_tx` 发送，同时写本地 WAV（`wav_path`，回退/留存用）。
    /// `level` 供 UI 绘制实时波形。
    pub fn start(
        audio_tx: Sender<Vec<u8>>,
        wav_path: PathBuf,
        level: LevelMeter,
    ) -> Result<Self, AudioError> {
        let cap = open_capture()?;
        let writer = create_writer(&wav_path)?;
        let resampler = MonoResampler::new(cap.input_rate)?;
        let stop_flag = Arc::new(AtomicBool::new(false));
        let worker = spawn_stream_worker(
            cap.cons,
            resampler,
            writer,
            cap.channels as usize,
            wav_path,
            audio_tx,
            stop_flag.clone(),
            level,
        );
        Ok(Self {
            stream: cap.stream,
            stop_flag,
            worker: Some(worker),
        })
    }

    /// 停止采集：暂停流 → worker 排空收尾、关闭音频通道（触发后端 finish-task）。
    pub fn stop(mut self) -> Result<RecordingOutcome, AudioError> {
        let _ = self.stream.pause();
        self.stop_flag.store(true, Ordering::SeqCst);
        match self.worker.take() {
            Some(handle) => handle.join().map_err(|_| AudioError::WorkerPanicked)?,
            None => Err(AudioError::WorkerPanicked),
        }
    }
}

impl Drop for StreamingCapture {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::SeqCst);
    }
}

#[allow(clippy::too_many_arguments)] // 内部 spawn 助手：参数皆为采集管线一次性入参
fn spawn_stream_worker(
    mut cons: AudioCons,
    mut resampler: MonoResampler,
    mut writer: WavSink,
    channels: usize,
    wav_path: PathBuf,
    audio_tx: Sender<Vec<u8>>,
    stop_flag: Arc<AtomicBool>,
    level: LevelMeter,
) -> JoinHandle<Result<RecordingOutcome, AudioError>> {
    std::thread::spawn(move || -> Result<RecordingOutcome, AudioError> {
        let started = Instant::now();
        let mut interleaved: Vec<f32> = Vec::with_capacity(8192);
        let mut mono: Vec<f32> = Vec::with_capacity(4096);
        let mut pcm: Vec<i16> = Vec::with_capacity(4096);
        let mut frame_bytes: Vec<u8> = Vec::with_capacity(FRAME_BYTES * 2);
        let mut chunk = vec![0f32; 4096];
        let mut total: u64 = 0;

        loop {
            let n = cons.pop_slice(&mut chunk);
            if n > 0 {
                super::store_level(&level, super::rms_amplitude(&chunk[..n]));
                interleaved.extend_from_slice(&chunk[..n]);
                pcm.clear();
                downmix_resample(&mut interleaved, channels, &mut resampler, &mut mono, &mut pcm)?;
                write_and_frame(&pcm, &mut writer, &mut frame_bytes, &audio_tx)?;
                total += pcm.len() as u64;
            } else if stop_flag.load(Ordering::SeqCst) {
                break;
            } else {
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        // 排空重采样器内部延迟样本。
        pcm.clear();
        resampler.flush(&mut pcm)?;
        write_and_frame(&pcm, &mut writer, &mut frame_bytes, &audio_tx)?;
        total += pcm.len() as u64;

        // 末尾不足 100ms 的残余也发出。
        if !frame_bytes.is_empty() {
            let _ = audio_tx.try_send(std::mem::take(&mut frame_bytes));
        }
        // 丢弃发送端 → 关闭通道 → 后端收到 None → 发 finish-task。
        drop(audio_tx);

        writer
            .finalize()
            .map_err(|e| AudioError::Wav(e.to_string()))?;
        Ok(RecordingOutcome {
            path: wav_path,
            frames: total,
            duration: started.elapsed(),
        })
    })
}

/// 写 WAV（始终，保证回退 WAV 完整）并按 100ms 帧 `try_send`（满则丢弃，不阻塞采集）。
fn write_and_frame(
    pcm: &[i16],
    writer: &mut WavSink,
    frame_bytes: &mut Vec<u8>,
    audio_tx: &Sender<Vec<u8>>,
) -> Result<(), AudioError> {
    for &sample in pcm {
        writer
            .write_sample(sample)
            .map_err(|e| AudioError::Wav(e.to_string()))?;
        frame_bytes.extend_from_slice(&sample.to_le_bytes());
    }
    while frame_bytes.len() >= FRAME_BYTES {
        let frame: Vec<u8> = frame_bytes.drain(..FRAME_BYTES).collect();
        // 通道满（重连/后端落后）则丢弃该帧——WAV 完整，回退离线无损。
        let _ = audio_tx.try_send(frame);
    }
    Ok(())
}

/// 降混完整帧为单声道并重采样为 i16（与 capture.rs 的 drain_frames 同逻辑，但不写 WAV）。
fn downmix_resample(
    interleaved: &mut Vec<f32>,
    channels: usize,
    resampler: &mut MonoResampler,
    mono: &mut Vec<f32>,
    out: &mut Vec<i16>,
) -> Result<(), AudioError> {
    let frames = interleaved.len() / channels;
    if frames == 0 {
        return Ok(());
    }
    mono.clear();
    for f in 0..frames {
        let base = f * channels;
        let sum: f32 = interleaved[base..base + channels].iter().sum();
        mono.push(sum / channels as f32);
    }
    interleaved.drain(..frames * channels);
    resampler.push(mono, out)?;
    Ok(())
}
