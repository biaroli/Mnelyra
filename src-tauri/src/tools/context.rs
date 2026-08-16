use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::activity::{ActivityCoordinator, ActivityGuard, ActivityKind};
use crate::harness::Harness;
use crate::tools::policy::PolicySettings;
use crate::tools::session::SessionStore;
use crate::tools::workspace::{relative_display, Workspace, WorkspaceError};
use crate::workspace::AuthConfig;

pub struct ToolContext {
    pub workspace_id: String,
    pub workspace: Workspace,
    pub auth: AuthConfig,
    pub policy: PolicySettings,
    pub tool_profile: String,
    pub permission_mode: String,
    pub harness: Harness,
    default_cwd: Mutex<PathBuf>,
    pub sessions: Arc<SessionStore>,
    activity: Option<ActivityCoordinator>,
}

pub type SharedToolContext = Arc<ToolContext>;

impl ToolContext {
    pub fn new(workspace_path: PathBuf) -> Result<Self, String> {
        let workspace = Workspace::new(workspace_path).map_err(|e| e.message())?;
        let auth = AuthConfig {
            auth_type: "internal".into(),
            ..AuthConfig::default()
        };
        Ok(Self::from_workspace(
            workspace,
            auth,
            PolicySettings::default(),
            "full".into(),
            "trusted".into(),
        ))
    }

    pub fn from_workspace(
        workspace: Workspace,
        auth: AuthConfig,
        policy: PolicySettings,
        tool_profile: String,
        permission_mode: String,
    ) -> Self {
        let workspace_id = workspace.root_display();
        let harness_root = Harness::default_root().expect("无法初始化 Harness 数据目录");
        Self::from_workspace_scoped_with_harness_root(
            workspace,
            auth,
            policy,
            crate::tools::registry::normalize_tool_profile(&tool_profile).into(),
            permission_mode,
            harness_root,
            workspace_id,
            None,
        )
    }

    pub fn from_workspace_scoped(
        workspace: Workspace,
        auth: AuthConfig,
        policy: PolicySettings,
        tool_profile: String,
        permission_mode: String,
        workspace_id: String,
        activity: ActivityCoordinator,
    ) -> Self {
        let harness_root = Harness::default_root().expect("无法初始化 Harness 数据目录");
        Self::from_workspace_scoped_with_harness_root(
            workspace,
            auth,
            policy,
            crate::tools::registry::normalize_tool_profile(&tool_profile).into(),
            permission_mode,
            harness_root,
            workspace_id,
            Some(activity),
        )
    }

    pub fn from_workspace_with_harness_root(
        workspace: Workspace,
        auth: AuthConfig,
        policy: PolicySettings,
        tool_profile: String,
        permission_mode: String,
        harness_root: PathBuf,
    ) -> Self {
        let workspace_id = workspace.root_display();
        Self::from_workspace_scoped_with_harness_root(
            workspace,
            auth,
            policy,
            tool_profile,
            permission_mode,
            harness_root,
            workspace_id,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_workspace_scoped_with_harness_root(
        workspace: Workspace,
        auth: AuthConfig,
        policy: PolicySettings,
        tool_profile: String,
        permission_mode: String,
        harness_root: PathBuf,
        workspace_id: String,
        activity: Option<ActivityCoordinator>,
    ) -> Self {
        let root = workspace.root().to_path_buf();
        Self {
            workspace_id,
            workspace,
            auth,
            policy,
            tool_profile: crate::tools::registry::normalize_tool_profile(&tool_profile).into(),
            permission_mode,
            harness: Harness::new(root.clone(), harness_root).expect("无法初始化 Harness"),
            default_cwd: Mutex::new(root),
            sessions: Arc::new(SessionStore::new()),
            activity,
        }
    }

    pub fn for_test(workspace_path: PathBuf, harness_root: PathBuf) -> Result<Self, String> {
        let workspace = Workspace::new(workspace_path).map_err(|e| e.message())?;
        Ok(Self::from_workspace_with_harness_root(
            workspace,
            AuthConfig {
                auth_type: "internal".into(),
                ..AuthConfig::default()
            },
            PolicySettings::default(),
            "full".into(),
            "trusted".into(),
            harness_root,
        ))
    }

    pub fn workspace_path(&self) -> String {
        self.workspace.root_display()
    }

    pub fn default_cwd_display(&self) -> String {
        let cwd = self.default_cwd.lock().expect("cwd lock");
        relative_display(self.workspace.root(), &cwd)
    }

    pub fn set_default_cwd(&self, path: PathBuf) {
        *self.default_cwd.lock().expect("cwd lock") = path;
    }

    pub fn default_cwd_path(&self) -> PathBuf {
        self.default_cwd.lock().expect("cwd lock").clone()
    }

    pub fn acquire_activity(
        &self,
        kind: ActivityKind,
    ) -> Result<Option<ActivityGuard>, WorkspaceError> {
        let Some(activity) = &self.activity else {
            return Ok(None);
        };
        activity
            .acquire(&self.workspace_id, kind)
            .map(Some)
            .map_err(|error| WorkspaceError::ToolDetails {
                code: "WORKSPACE_DRAINING",
                message: error.to_string(),
                category: "runtime",
                retryable: true,
                details: serde_json::json!({
                    "reason": "workspace_draining",
                    "workspace_id": self.workspace_id,
                    "retryable": true,
                    "suggestion": "Wait for the workspace switch to finish, then retry."
                }),
            })
    }
}
