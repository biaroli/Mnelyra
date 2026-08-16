use std::sync::Mutex;

use tokio::sync::Mutex as AsyncMutex;

use crate::activity::{ActiveWorkspacePhase, ActiveWorkspaceState, ActivityCoordinator};
use crate::data::DataStore;
use crate::error::AppResult;
use crate::provider::{CodexAppServerManager, ProviderRegistry, ProviderStatus};
use crate::runtime::RuntimeSupervisor;
use crate::session::{SessionCoordinator, SessionEventStore, TaskSessionStore};
use crate::tunnel::OpenAiTunnelManager;

pub struct AppState {
    pub data: Mutex<DataStore>,
    pub runtime: Mutex<RuntimeSupervisor>,
    pub activity: ActivityCoordinator,
    pub providers: ProviderRegistry,
    pub codex_app_server: CodexAppServerManager,
    pub openai_tunnel: OpenAiTunnelManager,
    pub sessions: TaskSessionStore,
    pub session_coordinator: SessionCoordinator,
    active_workspace: Mutex<ActiveWorkspaceState>,
    pub workspace_activation: AsyncMutex<()>,
}

impl AppState {
    pub fn new() -> AppResult<Self> {
        let mut store = DataStore::load()?;
        store.init_shared_secrets()?;
        let activity = ActivityCoordinator::default();
        let permission_ceiling = match store.settings().general.permission_ceiling.as_str() {
            "read_only" => "read_only",
            "custom" => "custom",
            _ => "automatic",
        }
        .to_string();
        let codex_app_server = CodexAppServerManager::with_permission_mode(permission_ceiling);
        let openai_tunnel = OpenAiTunnelManager::default();
        let providers = ProviderRegistry::with_defaults(codex_app_server.clone());
        let sessions = TaskSessionStore::load()?;
        let session_events = SessionEventStore::default();
        let session_coordinator = SessionCoordinator::new(
            codex_app_server.clone(),
            sessions.clone(),
            session_events.clone(),
            activity.clone(),
        );
        Ok(Self {
            data: Mutex::new(store),
            runtime: Mutex::new(RuntimeSupervisor::default()),
            activity,
            providers,
            codex_app_server,
            openai_tunnel,
            sessions,
            session_coordinator,
            active_workspace: Mutex::new(ActiveWorkspaceState::default()),
            workspace_activation: AsyncMutex::new(()),
        })
    }

    pub fn with_data<R>(&self, f: impl FnOnce(&mut DataStore) -> AppResult<R>) -> AppResult<R> {
        let mut guard = self
            .data
            .lock()
            .map_err(|_| crate::error::AppError::Message("data store poisoned".into()))?;
        f(&mut guard)
    }

    pub fn with_workspaces<R>(
        &self,
        f: impl FnOnce(&mut DataStore) -> AppResult<R>,
    ) -> AppResult<R> {
        self.with_data(f)
    }

    pub fn with_settings<R>(&self, f: impl FnOnce(&mut DataStore) -> AppResult<R>) -> AppResult<R> {
        self.with_data(f)
    }

    pub fn with_runtime<R>(
        &self,
        f: impl FnOnce(&mut RuntimeSupervisor) -> AppResult<R>,
    ) -> AppResult<R> {
        let mut guard = self
            .runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        f(&mut guard)
    }

    pub fn active_workspace_state(&self) -> AppResult<ActiveWorkspaceState> {
        let guard = self.active_workspace.lock().map_err(|_| {
            crate::error::AppError::Message("active workspace state poisoned".into())
        })?;
        Ok(guard.clone())
    }

    pub fn transition_active_workspace(
        &self,
        workspace_id: Option<String>,
        phase: ActiveWorkspacePhase,
        message: Option<String>,
    ) -> AppResult<ActiveWorkspaceState> {
        let mut guard = self.active_workspace.lock().map_err(|_| {
            crate::error::AppError::Message("active workspace state poisoned".into())
        })?;
        guard.transition(workspace_id, phase, message);
        Ok(guard.clone())
    }

    pub fn set_active_workspace(
        &self,
        workspace_id: impl Into<String>,
    ) -> AppResult<ActiveWorkspaceState> {
        let mut guard = self.active_workspace.lock().map_err(|_| {
            crate::error::AppError::Message("active workspace state poisoned".into())
        })?;
        guard.set_active(workspace_id);
        Ok(guard.clone())
    }

    pub async fn refresh_provider_activity(&self, workspace_id: &str) -> AppResult<ProviderStatus> {
        let status = self.providers.status("codex").await?;
        // Native Codex turns are owned by SessionCoordinator activity guards.
        // The provider status is global process health, not workspace-scoped
        // activity evidence, so do not attribute its turn count to an arbitrary
        // workspace here.
        self.activity
            .set_provider_activity(workspace_id, 0, 0, true);
        Ok(status)
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new().expect("failed to initialize app state")
    }
}
