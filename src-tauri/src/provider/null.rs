use super::{
    ProviderAdapter, ProviderCapability, ProviderDescriptor, ProviderFuture, ProviderState,
    ProviderStatus,
};

#[derive(Clone)]
pub struct NullProvider {
    status: ProviderStatus,
}

impl Default for NullProvider {
    fn default() -> Self {
        Self {
            status: ProviderStatus {
                provider_id: "null".into(),
                state: ProviderState::Ready,
                configured: true,
                activity_known: true,
                version: Some("test".into()),
                mode: Some("test".into()),
                pid: None,
                accepting_tasks: Some(true),
                active_turns: 0,
                active_http_turns: 0,
                active_browser_turns: 0,
                session_ready: Some(true),
                message: "Null provider ready".into(),
            },
        }
    }
}

impl ProviderAdapter for NullProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: "null".into(),
            name: "Null Provider".into(),
            capabilities: vec![ProviderCapability::Status],
        }
    }

    fn status(&self) -> ProviderFuture<'_, ProviderStatus> {
        let status = self.status.clone();
        Box::pin(async move { status })
    }
}
