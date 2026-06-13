/*use qwen3_asr::{best_device, AsrInference, TranscribeOptions};
use std::path::Path;
use std::time::Instant;
// 引入 Instant 用于耗时测量

fn main() -> Result<(), Box<dyn std::error::Error>> {
    unsafe {
        std::env::set_var("https_proxy", "http://127.0.0.1:10808");
        std::env::set_var("http_proxy", "http://127.0.0.1:10808");
        std::env::set_var("all_proxy", "http://127.0.0.1:10808");
    }

    let device = best_device(); // automatically selects CUDA → Metal → CPU

    println!("device: {:?}", device);

    // 加载 Qwen3-ASR-0.6B 的 safetensors 权重（推荐 0.6B 用于 CPU 推理）
    // let engine = AsrInference::load(Path::new("D:\\llm\\models\\Qwen\\Qwen3-ASR-0.6B"), device)?;
    let engine =
        AsrInference::from_pretrained("Qwen/Qwen3-ASR-0.6B", Path::new("models/"), device)?;

    // 1. 在执行推理前记录起始时间
    let start_time = Instant::now();

    println!("now1: {:?}", start_time.elapsed());

    // 执行推理
    let result = engine.transcribe(
        "C:\\Users\\huang\\AppData\\Local\\Temp\\voxink_recording_20260613_152301.wav",
        TranscribeOptions::default(),
    )?;

    println!("now2: {:?}", start_time.elapsed());

    let result = engine.transcribe(
        "C:\\Users\\huang\\AppData\\Local\\Temp\\voxink_recording_20260613_152325.wav",
        TranscribeOptions::default(),
    )?;
    println!("now3: {:?}", start_time.elapsed());

    // 2. 推理结束后计算耗时
    let duration = start_time.elapsed();

    println!("识别语言: {}", result.language);
    println!("识别文本: {}", result.text);

    // 3. 输出耗时信息（提供秒和毫秒两种精度，方便观察）
    println!("----------------------------------------");
    println!(
        "ASR 推理耗时: {:.2} 秒 ({} 毫秒)",
        duration.as_secs_f64(),
        duration.as_millis()
    );
    println!("----------------------------------------");

    Ok(())
}
*/
fn main() {
    println!("Running test suite...");
}
