//! 后端注册表（§2.6）。
//!
//! 工厂模式：注册 `backend_id → 创建闭包` 映射；运行时按 id 取实例、枚举后端及能力。
//! 应用层只依赖 `AsrBackend` trait，不直接 import 具体后端模块。

use std::collections::HashMap;
use std::sync::Arc;

use super::backends::bailian_filetrans::BailianFiletransBackend;
use super::backends::bailian_offline::BailianOfflineBackend;
use super::backends::bailian_streaming::BailianStreamingBackend;
use super::traits::AsrBackend;

type BackendFactory = Box<dyn Fn() -> Arc<dyn AsrBackend> + Send + Sync>;

/// 后端能力概要（用于设置面板枚举展示，M7 使用）。
#[derive(Debug, Clone)]
#[allow(dead_code)] // 字段在 M7 设置面板枚举后端时读取
pub struct BackendInfo {
    pub id: String,
    pub display_name: String,
    pub supports_streaming: bool,
    pub supports_offline: bool,
}

/// 后端注册表。内部数据结构（HashMap）属实现细节。
#[derive(Default)]
pub struct BackendRegistry {
    factories: HashMap<String, BackendFactory>,
}

impl BackendRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册全部内置后端（§2.6 清单中已落地的部分）。
    /// M4 仅有 `aliyun_bailian_offline`；M6/M7/M8 逐步补齐其余。
    pub fn with_builtins() -> Self {
        let mut registry = Self::new();
        registry.register("aliyun_bailian_offline", || {
            Arc::new(BailianOfflineBackend::new())
        });
        registry.register("aliyun_bailian_filetrans", || {
            Arc::new(BailianFiletransBackend::new())
        });
        registry.register("aliyun_bailian_streaming", || {
            Arc::new(BailianStreamingBackend::new())
        });
        registry
    }

    pub fn register(
        &mut self,
        id: impl Into<String>,
        factory: impl Fn() -> Arc<dyn AsrBackend> + Send + Sync + 'static,
    ) {
        self.factories.insert(id.into(), Box::new(factory));
    }

    /// 按 id 取后端实例。
    pub fn get(&self, id: &str) -> Option<Arc<dyn AsrBackend>> {
        self.factories.get(id).map(|factory| factory())
    }

    /// 枚举所有已注册后端及其能力（M7 设置面板使用）。
    #[allow(dead_code)] // M7 连接测试/后端选择 UI 使用
    pub fn list(&self) -> Vec<BackendInfo> {
        self.factories
            .values()
            .map(|factory| {
                let backend = factory();
                BackendInfo {
                    id: backend.backend_id().to_string(),
                    display_name: backend.display_name().to_string(),
                    supports_streaming: backend.supports_streaming(),
                    supports_offline: backend.supports_offline(),
                }
            })
            .collect()
    }
}
