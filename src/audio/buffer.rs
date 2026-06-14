//! 无锁环形缓冲区（任务 3.2）。
//!
//! 单生产者（cpal 音频回调）→ 单消费者（worker 线程）。缓冲区中存放**交织的**
//! 输入采样（已转换为 f32），由消费者负责降混与重采样。

use ringbuf::traits::Split;
use ringbuf::{HeapCons, HeapProd, HeapRb};

/// 生产者句柄（移动进 cpal 回调）。
pub type AudioProd = HeapProd<f32>;
/// 消费者句柄（移动进 worker 线程）。
pub type AudioCons = HeapCons<f32>;

/// 创建容量为 `capacity` 个 f32 样本的环形缓冲区。
pub fn new_buffer(capacity: usize) -> (AudioProd, AudioCons) {
    HeapRb::<f32>::new(capacity).split()
}
