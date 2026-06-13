//! ASR 后端子系统 —— §2.2/2.3/2.5/2.6/§7。
//!
//! 契约层（traits/error/config/registry）在 M4 一次落地，后续 M6/M7/M8 直接面向 trait
//! 实现新后端，避免大重构。应用层只依赖 `AsrBackend` trait 与注册表。

pub mod backends;
pub mod client;
pub mod config;
pub mod error;
pub mod oss;
pub mod registry;
pub mod traits;

pub use config::AsrConfig;
pub use error::AsrError;
pub use registry::BackendRegistry;
