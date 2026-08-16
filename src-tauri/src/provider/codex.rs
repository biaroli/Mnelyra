use super::{
    CodexAppServerManager, ProviderAdapter, ProviderCapability, ProviderDescriptor, ProviderFuture,
    ProviderState, ProviderStatus,
};

const PROVIDER_ID: &str = "codex";

#[derive(Clone)]
pub struct CodexNativeAdapter {
    app_server: CodexAppServerManager,
}

impl CodexNativeAdapter {
    pub fn new(app_server: CodexAppServerManager) -> Self {
        Self { app_server }
    }

    async fn probe(&self) -> ProviderStatus {
        let runtime = self.app_server.runtime_status().await;
        if !runtime.available {
            return ProviderStatus::not_configured(PROVIDER_ID, runtime.message);
        }

        ProviderStatus {
            provider_id: PROVIDER_ID.into(),
            state: if runtime.active_turns > 0 {
                ProviderState::Busy
            } else {
                ProviderState::Ready
            },
            configured: true,
            activity_known: true,
            version: None,
            mode: Some("app-server".into()),
            pid: runtime.pid,
            accepting_tasks: Some(runtime.running || runtime.pid.is_none()),
            active_turns: runtime.active_turns,
            active_http_turns: 0,
            active_browser_turns: 0,
            session_ready: Some(true),
            message: runtime.message,
        }
    }
}

impl ProviderAdapter for CodexNativeAdapter {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: PROVIDER_ID.into(),
            name: "Codex".into(),
            capabilities: vec![
                ProviderCapability::Status,
                ProviderCapability::Sessions,
                ProviderCapability::StartTask,
                ProviderCapability::SendInput,
                ProviderCapability::CancelTask,
                ProviderCapability::Compaction,
                ProviderCapability::Resume,
            ],
        }
    }

    fn status(&self) -> ProviderFuture<'_, ProviderStatus> {
        Box::pin(self.probe())
    }
}
