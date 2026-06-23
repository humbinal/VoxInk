//! 应用内音频回放 —— 片段试听（IDEAS：片段播放改为应用内实现）。
//!
//! 基于 rodio（纯 Rust，底层走 cpal/WASAPI，无 C 依赖）。用 rodio 的流式 [`Decoder`]
//! （`wav` 特性走 hound、`mp3` 特性走 symphonia）**边读边解**：内存恒定，不随文件大小膨胀
//! ——避免大文件（录音上限 10 小时 ≈ 1.15GB）整段解码进内存。Decoder 自动按内容探测容器
//! （wav/mp3），并完成与输出设备的重采样/声道映射。mp3 流式无法预知总时长，故由调用方传入
//! 片段时长作回退（用于进度条）。
//!
//! ⚠️ [`OutputStream`] 是 `!Send` 且 **drop 即停**，必须随播放存活（存入视图字段）。
//! 流式回放期间会持有该文件的只读句柄，drop [`Player`]（停播/切段/删除）即关闭。

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};

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
    /// 打开并立即开始流式播放一个音频文件（wav/mp3，不整段解码进内存）。
    /// `fallback_secs` 为片段已知时长，用于解码器无法给出总时长（如 mp3 流式）时的进度条回退。
    pub fn play(path: &Path, fallback_secs: u32) -> Result<Self> {
        let (stream, handle) = OutputStream::try_default().context("打开音频输出设备失败")?;
        let sink = Sink::try_new(&handle).context("创建播放 Sink 失败")?;

        let file = BufReader::new(File::open(path).context("打开音频文件失败")?);
        let source = Decoder::new(file).context("解码音频失败")?;
        // wav 头直接给出总时长；mp3 等流式拿不到时回退到片段已知时长（再缺则 0，进度条隐藏）。
        let total = source
            .total_duration()
            .unwrap_or_else(|| Duration::from_secs(fallback_secs as u64));

        sink.append(source);
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
