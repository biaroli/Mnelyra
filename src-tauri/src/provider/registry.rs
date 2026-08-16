use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::error::{AppError, AppResult};

use super::{
    CodexAppServerManager, CodexNativeAdapter, ProviderAdapter, ProviderDescriptor, ProviderStatus,
};

#[derive(Clone, Default)]
pub struct ProviderRegistry {
    inner: Arc<RwLock<BTreeMap<String, Arc<dyn ProviderAdapter>>>>,
}

impl ProviderRegistry {
    pub fn with_defaults(codex_app_server: CodexAppServerManager) -> Self {
        let registry = Self::default();
        registry.register(Arc::new(CodexNativeAdapter::new(codex_app_server)));
        registry
    }

    pub fn register(&self, adapter: Arc<dyn ProviderAdapter>) {
        let descriptor = adapter.descriptor();
        self.inner
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(descriptor.id, adapter);
    }

    pub fn descriptors(&self) -> Vec<ProviderDescriptor> {
        self.inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .map(|adapter| adapter.descriptor())
            .collect()
    }

    pub async fn status(&self, id: &str) -> AppResult<ProviderStatus> {
        let adapter = self
            .inner
            .read()
            .map_err(|_| AppError::Message("provider registry poisoned".into()))?
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::Message(format!("provider not found: {id}")))?;
        Ok(adapter.status().await)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::provider::{NullProvider, ProviderState};

    #[tokio::test]
    async fn null_provider_normalizes_status_without_external_runtime() {
        let registry = ProviderRegistry::default();
        registry.register(Arc::new(NullProvider::default()));
        let descriptors = registry.descriptors();
        assert_eq!(descriptors.len(), 1);
        assert_eq!(descriptors[0].id, "null");
        let status = registry.status("null").await.expect("null provider status");
        assert_eq!(status.state, ProviderState::Ready);
        assert_eq!(status.active_turns, 0);
    }
}
