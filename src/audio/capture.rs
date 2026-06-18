//! 音频采集与录音控制（任务 3.1 / 3.3）。
//!
//! `Recorder::start()` 在主线程构建 cpal 输入流并启动 worker 线程；`stop()` 暂停采集、
//! 通知 worker 排空缓冲、收尾 WAV 并返回产物。cpal `Stream` 不跨线程移动（留在主线程）。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SizedSample};
use ringbuf::traits::{Consumer, Producer};

use super::buffer::{AudioCons, AudioProd, new_buffer};
use super::resample::MonoResampler;
use super::writer::{WavSink, create_writer};
use super::{AudioError, LevelMeter, RecordingOutcome};

/// 录音会话句柄。持有 cpal 流（主线程）、停止标志与 worker 线程句柄。
pub struct Recorder {
    stream: cpal::Stream,
    stop_flag: Arc<AtomicBool>,
    worker: Option<JoinHandle<Result<u64, AudioError>>>,
    wav_path: PathBuf,
    started_at: Instant,
}

/// 已就绪的采集句柄：cpal 流（需保持存活）+ 消费端 + 实际输入参数。
/// WAV 录音（M3）与实时流式（M6）两条路径共用。
pub(crate) struct OpenCapture {
    pub stream: cpal::Stream,
    pub cons: AudioCons,
    pub input_rate: u32,
    pub channels: u16,
}

/// 探测默认输入设备、建环形缓冲、构建并启动采集流。
pub(crate) fn open_capture() -> Result<OpenCapture, AudioError> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or(AudioError::NoInputDevice)?;
    let device_name = device.name().unwrap_or_else(|_| "<unknown>".to_string());

    let supported = device
        .default_input_config()
        .map_err(|e| AudioError::DefaultConfig(e.to_string()))?;
    let sample_format = supported.sample_format();
    let input_rate = supported.sample_rate().0;
    let channels = supported.channels();
    let stream_config: cpal::StreamConfig = supported.into();

    // 录制开始时打印实际音频配置与重采样参数（§12.4）。
    tracing::info!(
        device = %device_name,
        ?sample_format,
        input_rate,
        channels,
        target_rate = super::TARGET_SAMPLE_RATE,
        "开始采集：实际音频输入配置"
    );

    // 环形缓冲区：约 2 秒输入交织样本（至少 4096）。
    let capacity = (input_rate as usize * channels as usize * 2).max(4096);
    let (prod, cons) = new_buffer(capacity);
    let stream = build_stream(&device, &stream_config, sample_format, prod)?;
    stream
        .play()
        .map_err(|e| AudioError::PlayStream(e.to_string()))?;

    Ok(OpenCapture {
        stream,
        cons,
        input_rate,
        channels,
    })
}

impl Recorder {
    /// 探测默认输入设备、构建采集流并开始录音（WAV 写入 `wav_path`）。
    /// 路径由调用方决定（持久化时为记录目录，否则为临时目录）；`level` 供 UI 绘制实时波形。
    pub fn start(wav_path: PathBuf, level: LevelMeter) -> Result<Self, AudioError> {
        let cap = open_capture()?;
        let writer = create_writer(&wav_path)?;
        let resampler = MonoResampler::new(cap.input_rate)?;

        let stop_flag = Arc::new(AtomicBool::new(false));
        let worker = spawn_worker(
            cap.cons,
            resampler,
            writer,
            cap.channels as usize,
            stop_flag.clone(),
            level,
        );

        Ok(Self {
            stream: cap.stream,
            stop_flag,
            worker: Some(worker),
            wav_path,
            started_at: Instant::now(),
        })
    }

    /// 停止录音：暂停采集 → 通知 worker 排空收尾 → 返回产物。
    pub fn stop(mut self) -> Result<RecordingOutcome, AudioError> {
        // 先停止采集回调，避免 worker 排空时仍有新样本进来。
        let _ = self.stream.pause();
        self.stop_flag.store(true, Ordering::SeqCst);

        let frames = match self.worker.take() {
            Some(handle) => handle.join().map_err(|_| AudioError::WorkerPanicked)??,
            None => 0,
        };

        let outcome = RecordingOutcome {
            path: self.wav_path.clone(),
            frames,
            duration: self.started_at.elapsed(),
        };
        tracing::info!(
            path = %outcome.path.display(),
            frames = outcome.frames,
            secs = outcome.duration.as_secs(),
            "录音结束，WAV 已生成"
        );
        Ok(outcome)
    }

    /// 生成中的 WAV 路径（M4 离线上传会用到）。
    #[allow(dead_code)] // M4 离线 ASR 上传时读取
    pub fn wav_path(&self) -> &Path {
        &self.wav_path
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        // 未经 stop() 直接丢弃（如录音中退出）时，通知 worker 结束以免线程泄漏。
        self.stop_flag.store(true, Ordering::SeqCst);
    }
}

/// 按设备采样格式分派到泛型构建函数。
fn build_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    format: cpal::SampleFormat,
    prod: AudioProd,
) -> Result<cpal::Stream, AudioError> {
    use cpal::SampleFormat::*;
    match format {
        F32 => build_typed::<f32>(device, config, prod),
        F64 => build_typed::<f64>(device, config, prod),
        I8 => build_typed::<i8>(device, config, prod),
        I16 => build_typed::<i16>(device, config, prod),
        I32 => build_typed::<i32>(device, config, prod),
        U8 => build_typed::<u8>(device, config, prod),
        U16 => build_typed::<u16>(device, config, prod),
        U32 => build_typed::<u32>(device, config, prod),
        other => Err(AudioError::UnsupportedFormat(other.to_string())),
    }
}

/// 构建某具体采样类型 `T` 的输入流：回调中把 `T` 转为 f32 写入环形缓冲区。
fn build_typed<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut prod: AudioProd,
) -> Result<cpal::Stream, AudioError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    // 回调内复用的转换缓冲区（clear 保留容量，预热后不再分配）。
    let mut scratch: Vec<f32> = Vec::new();
    let err_fn = |err| tracing::error!("音频输入流错误: {err}");

    let stream = device
        .build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                // 仅做轻量格式转换 + 写缓冲区，绝不阻塞（§3.3）。
                scratch.clear();
                scratch.extend(data.iter().map(|&s| f32::from_sample(s)));
                // 缓冲区满时多余样本被丢弃（消费端落后）；回调内不做 I/O 日志。
                let _ = prod.push_slice(&scratch);
            },
            err_fn,
            None,
        )
        .map_err(|e| AudioError::BuildStream(e.to_string()))?;
    Ok(stream)
}

/// 启动 worker 线程：读环形缓冲 → 降混单声道 → 重采样 → 写 WAV。返回写入的样本数。
fn spawn_worker(
    mut cons: AudioCons,
    mut resampler: MonoResampler,
    mut writer: WavSink,
    channels: usize,
    stop_flag: Arc<AtomicBool>,
    level: LevelMeter,
) -> JoinHandle<Result<u64, AudioError>> {
    std::thread::spawn(move || -> Result<u64, AudioError> {
        let mut interleaved: Vec<f32> = Vec::with_capacity(8192);
        let mut mono: Vec<f32> = Vec::with_capacity(4096);
        let mut pcm: Vec<i16> = Vec::with_capacity(4096);
        let mut chunk = vec![0f32; 4096];
        let mut total: u64 = 0;

        loop {
            let n = cons.pop_slice(&mut chunk);
            if n > 0 {
                // 更新实时电平（取本次采集块 RMS），供 UI 绘制波形。
                super::store_level(&level, super::rms_amplitude(&chunk[..n]));
                interleaved.extend_from_slice(&chunk[..n]);
                total += drain_frames(
                    &mut interleaved,
                    channels,
                    &mut resampler,
                    &mut writer,
                    &mut mono,
                    &mut pcm,
                )?;
            } else if stop_flag.load(Ordering::SeqCst) {
                // 缓冲区已空且收到停止信号：排空重采样器内部延迟样本后收尾。
                break;
            } else {
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        pcm.clear();
        resampler.flush(&mut pcm)?;
        for &sample in pcm.iter() {
            writer
                .write_sample(sample)
                .map_err(|e| AudioError::Wav(e.to_string()))?;
        }
        total += pcm.len() as u64;

        writer
            .finalize()
            .map_err(|e| AudioError::Wav(e.to_string()))?;
        Ok(total)
    })
}

/// 把 `interleaved` 中的完整帧降混为单声道、重采样并写入；保留不足一帧的残余。
fn drain_frames(
    interleaved: &mut Vec<f32>,
    channels: usize,
    resampler: &mut MonoResampler,
    writer: &mut WavSink,
    mono: &mut Vec<f32>,
    pcm: &mut Vec<i16>,
) -> Result<u64, AudioError> {
    let frames = interleaved.len() / channels;
    if frames == 0 {
        return Ok(0);
    }

    mono.clear();
    for f in 0..frames {
        let base = f * channels;
        let sum: f32 = interleaved[base..base + channels].iter().sum();
        mono.push(sum / channels as f32);
    }
    interleaved.drain(..frames * channels);

    pcm.clear();
    resampler.push(mono, pcm)?;
    for &sample in pcm.iter() {
        writer
            .write_sample(sample)
            .map_err(|e| AudioError::Wav(e.to_string()))?;
    }
    Ok(pcm.len() as u64)
}
