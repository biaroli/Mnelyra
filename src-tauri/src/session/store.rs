use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::platform::platform;

use super::events::timestamp;
use super::{TaskSession, TaskSessionState};

#[derive(Debug, Serialize, Deserialize)]
struct SessionFile {
    version: u32,
    sessions: Vec<TaskSession>,
}

#[derive(Clone)]
pub struct TaskSessionStore {
    inner: Arc<RwLock<BTreeMap<String, TaskSession>>>,
    path: Option<Arc<PathBuf>>,
}

impl TaskSessionStore {
    pub fn load() -> AppResult<Self> {
        let path = platform()
            .app_config_dir()?
            .join("data")
            .join("sessions.json");
        let mut map = BTreeMap::new();
        for mut session in load_session_file(&path)? {
            // The previous desktop process can no longer prove an in-flight
            // turn survived. Keep the durable Codex thread binding, but reopen
            // the normalized session at an explicit user-controlled boundary.
            if matches!(
                session.state,
                TaskSessionState::Starting
                    | TaskSessionState::Running
                    | TaskSessionState::WaitingForTool
                    | TaskSessionState::Compacting
                    | TaskSessionState::Draining
            ) {
                session.state = TaskSessionState::WaitingForUser;
                session.updated_at = timestamp();
            }
            map.insert(session.id.clone(), session);
        }
        let store = Self {
            inner: Arc::new(RwLock::new(map)),
            path: Some(Arc::new(path)),
        };
        store.persist()?;
        Ok(store)
    }

    pub fn create(
        &self,
        workspace_id: impl Into<String>,
        canonical_workspace_path: impl Into<String>,
        provider_id: impl Into<String>,
        title: impl Into<String>,
    ) -> AppResult<TaskSession> {
        let now = timestamp();
        let session = TaskSession {
            id: uuid::Uuid::new_v4().to_string(),
            workspace_id: workspace_id.into(),
            canonical_workspace_path: canonical_workspace_path.into(),
            provider_id: provider_id.into(),
            provider_session_id: None,
            title: title.into(),
            state: TaskSessionState::Queued,
            created_at: now.clone(),
            updated_at: now.clone(),
            last_activity_at: now,
        };
        self.upsert(session.clone())?;
        Ok(session)
    }

    pub fn upsert(&self, session: TaskSession) -> AppResult<()> {
        let mut inner = self
            .inner
            .write()
            .map_err(|_| AppError::Message("task session store poisoned".into()))?;
        if let Some(existing) = inner.get(&session.id) {
            if existing.workspace_id != session.workspace_id
                || existing.canonical_workspace_path != session.canonical_workspace_path
                || existing.provider_id != session.provider_id
            {
                return Err(AppError::Message(format!(
                    "session {} binding is immutable",
                    session.id
                )));
            }
        }
        inner.insert(session.id.clone(), session);
        self.persist_map(&inner)
    }

    pub fn update_state(&self, id: &str, state: TaskSessionState) -> AppResult<TaskSession> {
        let mut inner = self
            .inner
            .write()
            .map_err(|_| AppError::Message("task session store poisoned".into()))?;
        let session = inner
            .get_mut(id)
            .ok_or_else(|| AppError::Message(format!("task session not found: {id}")))?;
        let now = timestamp();
        session.state = state;
        session.updated_at = now.clone();
        session.last_activity_at = now;
        let updated = session.clone();
        self.persist_map(&inner)?;
        Ok(updated)
    }

    pub fn bind_provider_session(
        &self,
        id: &str,
        provider_session_id: &str,
    ) -> AppResult<TaskSession> {
        let mut inner = self
            .inner
            .write()
            .map_err(|_| AppError::Message("task session store poisoned".into()))?;
        let session = inner
            .get_mut(id)
            .ok_or_else(|| AppError::Message(format!("task session not found: {id}")))?;
        if let Some(existing) = &session.provider_session_id {
            if existing != provider_session_id {
                return Err(AppError::Message(format!(
                    "session {id} is already bound to provider session {existing}"
                )));
            }
            return Ok(session.clone());
        }
        session.provider_session_id = Some(provider_session_id.to_string());
        let now = timestamp();
        session.updated_at = now.clone();
        session.last_activity_at = now;
        let updated = session.clone();
        self.persist_map(&inner)?;
        Ok(updated)
    }

    pub fn get(&self, id: &str) -> AppResult<TaskSession> {
        self.inner
            .read()
            .map_err(|_| AppError::Message("task session store poisoned".into()))?
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::Message(format!("task session not found: {id}")))
    }

    pub fn list(&self) -> AppResult<Vec<TaskSession>> {
        let inner = self
            .inner
            .read()
            .map_err(|_| AppError::Message("task session store poisoned".into()))?;
        let mut sessions = inner.values().cloned().collect::<Vec<_>>();
        sessions.sort_by(|a, b| {
            b.last_activity_at
                .cmp(&a.last_activity_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(sessions)
    }

    fn persist(&self) -> AppResult<()> {
        let inner = self
            .inner
            .read()
            .map_err(|_| AppError::Message("task session store poisoned".into()))?;
        self.persist_map(&inner)
    }

    fn persist_map(&self, map: &BTreeMap<String, TaskSession>) -> AppResult<()> {
        let Some(path) = self.path.as_deref() else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = SessionFile {
            version: 1,
            sessions: map.values().cloned().collect(),
        };
        let text = serde_json::to_string_pretty(&file)?;
        std::fs::write(path, format!("{text}\n"))?;
        Ok(())
    }
}

impl Default for TaskSessionStore {
    fn default() -> Self {
        Self {
            inner: Arc::new(RwLock::new(BTreeMap::new())),
            path: None,
        }
    }
}

fn load_session_file(path: &Path) -> AppResult<Vec<TaskSession>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(path)?;
    let file: SessionFile = serde_json::from_str(&raw).map_err(|error| {
        AppError::Message(format!(
            "invalid Mnelyra session store {}: {error}",
            path.display()
        ))
    })?;
    if file.version != 1 {
        return Err(AppError::Message(format!(
            "unsupported Mnelyra session store version {}",
            file.version
        )));
    }
    Ok(file.sessions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_binding_cannot_move_to_another_workspace() {
        let store = TaskSessionStore::default();
        let session = store
            .create("workspace-a", "E:\\WorkspaceA", "null", "test task")
            .expect("create session");
        let mut moved = session.clone();
        moved.workspace_id = "workspace-b".into();
        moved.canonical_workspace_path = "E:\\WorkspaceB".into();
        assert!(store.upsert(moved).is_err());
        assert_eq!(store.list().expect("list sessions"), vec![session]);
    }

    #[test]
    fn persistent_store_round_trips_provider_binding() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("sessions.json");
        let store = TaskSessionStore {
            inner: Arc::new(RwLock::new(BTreeMap::new())),
            path: Some(Arc::new(path.clone())),
        };
        let session = store
            .create("workspace-a", "E:\\WorkspaceA", "codex", "persisted")
            .expect("create");
        store
            .bind_provider_session(&session.id, "thr_123")
            .expect("bind");
        let sessions = load_session_file(&path).expect("load file");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].provider_session_id.as_deref(), Some("thr_123"));
    }
}
