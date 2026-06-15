//! 内置 ASR 后端实现。
//!
//! 各后端实现 `AsrBackend` trait（§2.2），通过注册表（§2.6）登记后供应用层按 id 获取。

pub mod bailian_filetrans;
pub mod bailian_offline;
pub mod bailian_streaming;
pub mod qwen3_asr_selfhosted;
