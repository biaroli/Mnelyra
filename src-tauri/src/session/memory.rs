use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::{AppError, AppResult};

use super::events::timestamp;
use super::TaskSession;

const CHECKPOINT_VERSION: u32 = 1;
const PROVIDER_OVERVIEW_LIMIT: usize = 80;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCheckpoint {
    pub version: u32,
    pub checkpoint_id: String,
    pub provider_id: String,
    pub mnelyra_session_id: String,
    pub workspace_id: String,
    pub canonical_workspace_path: String,
    pub provider_session_id: String,
    pub provider_turn_id: Option<String>,
    pub captured_at: String,
    pub source: String,
    pub content_sha256: String,
    pub thread_metadata: Value,
    pub turn_snapshot: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCheckpointSummary {
    pub checkpoint_id: String,
    pub provider_id: String,
    pub mnelyra_session_id: String,
    pub provider_session_id: String,
    pub provider_turn_id: Option<String>,
    pub captured_at: String,
    pub content_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceMemoryOverview {
    pub workspace_id: String,
    pub history_root: String,
    pub manifest_exists: bool,
    pub state_exists: bool,
    pub archive_revision: String,
    pub memory_revision: String,
    pub generated_at: String,
    pub current_focus: String,
    pub recent_changes: Vec<String>,
    pub open_items: Vec<String>,
    pub provider_checkpoint_count: usize,
    pub provider_checkpoints: Vec<ProviderCheckpointSummary>,
}

#[derive(Debug, Default, Deserialize)]
struct DerivedManifestView {
    #[serde(default)]
    archive_revision: String,
    #[serde(default)]
    memory_revision: String,
}

#[derive(Debug, Default, Deserialize)]
struct DerivedStateView {
    #[serde(default)]
    generated_at: String,
    #[serde(default)]
    current_focus: String,
    #[serde(default)]
    recent_changes: Vec<String>,
    #[serde(default)]
    open_items: Vec<String>,
}

pub(super) fn write_provider_checkpoint(
    session: &TaskSession,
    turn_id: Option<&str>,
    thread_read: &Value,
) -> AppResult<ProviderCheckpoint> {
    let provider_session_id = session.provider_session_id.clone().ok_or_else(|| {
        AppError::Message(format!("session {} has no provider session id", session.id))
    })?;
    let root = canonical_session_root(session)?;
    let directory = providers_dir(&root);
    std::fs::create_dir_all(&directory)?;

    let (thread_metadata, turn_snapshot) = split_thread_snapshot(thread_read, turn_id);
    let payload_for_hash = serde_json::to_vec(&serde_json::json!({
        "provider": session.provider_id,
        "mnelyraSessionId": session.id,
        "workspaceId": session.workspace_id,
        "providerSessionId": provider_session_id,
        "providerTurnId": turn_id,
        "threadMetadata": thread_metadata,
        "turnSnapshot": turn_snapshot,
    }))?;
    let content_sha256 = format!("{:x}", Sha256::digest(payload_for_hash));
    let checkpoint_id = uuid::Uuid::new_v4().to_string();
    let record = ProviderCheckpoint {
        version: CHECKPOINT_VERSION,
        checkpoint_id: checkpoint_id.clone(),
        provider_id: session.provider_id.clone(),
        mnelyra_session_id: session.id.clone(),
        workspace_id: session.workspace_id.clone(),
        canonical_workspace_path: root.to_string_lossy().into_owned(),
        provider_session_id,
        provider_turn_id: turn_id.map(str::to_string),
        captured_at: timestamp(),
        source: "codex-app-server:thread/read".into(),
        content_sha256,
        thread_metadata,
        turn_snapshot,
    };
    let path = directory.join(format!("{checkpoint_id}.json"));
    write_new_json(&path, &record)?;
    crate::tools::history::refresh_derived_memory(&root)
        .map_err(|error| AppError::Message(error.to_string()))?;
    Ok(record)
}

pub fn list_provider_checkpoints(
    workspace_id: &str,
    workspace_root: &Path,
) -> AppResult<WorkspaceMemoryOverview> {
    let root = std::fs::canonicalize(workspace_root)?;
    let history_root = root.join(".rootrelay").join("history-session");
    let memory_root = history_root.join("memory");
    let directory = memory_root.join("providers");
    let mut summaries = Vec::new();
    if directory.is_dir() {
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let Ok(record) = read_checkpoint_path(&path) else {
                continue;
            };
            summaries.push(summary(&record));
        }
    }
    summaries.sort_by(|a, b| b.captured_at.cmp(&a.captured_at));
    let provider_checkpoint_count = summaries.len();
    summaries.truncate(PROVIDER_OVERVIEW_LIMIT);
    let manifest_path = memory_root.join("manifest.json");
    let state_path = memory_root.join("state.json");
    let manifest = read_optional_json::<DerivedManifestView>(&manifest_path).unwrap_or_default();
    let state = read_optional_json::<DerivedStateView>(&state_path).unwrap_or_default();
    Ok(WorkspaceMemoryOverview {
        workspace_id: workspace_id.to_string(),
        history_root: history_root.to_string_lossy().into_owned(),
        manifest_exists: manifest_path.is_file(),
        state_exists: state_path.is_file(),
        archive_revision: manifest.archive_revision,
        memory_revision: manifest.memory_revision,
        generated_at: state.generated_at,
        current_focus: state.current_focus,
        recent_changes: state.recent_changes,
        open_items: state.open_items,
        provider_checkpoint_count,
        provider_checkpoints: summaries,
    })
}

fn read_optional_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn read_provider_checkpoint(
    workspace_root: &Path,
    checkpoint_id: &str,
) -> AppResult<ProviderCheckpoint> {
    validate_checkpoint_id(checkpoint_id)?;
    let root = std::fs::canonicalize(workspace_root)?;
    let path = providers_dir(&root).join(format!("{checkpoint_id}.json"));
    read_checkpoint_path(&path)
}

fn canonical_session_root(session: &TaskSession) -> AppResult<PathBuf> {
    let root = std::fs::canonicalize(&session.canonical_workspace_path)?;
    if !root.is_dir() {
        return Err(AppError::Message(format!(
            "session workspace is not a directory: {}",
            root.display()
        )));
    }
    Ok(root)
}

fn providers_dir(root: &Path) -> PathBuf {
    root.join(".rootrelay")
        .join("history-session")
        .join("memory")
        .join("providers")
}

fn split_thread_snapshot(thread_read: &Value, turn_id: Option<&str>) -> (Value, Value) {
    let thread = thread_read.get("thread").unwrap_or(thread_read);
    let mut metadata = thread.clone();
    let turns = metadata
        .as_object_mut()
        .and_then(|object| object.remove("turns"))
        .unwrap_or(Value::Null);
    let turn = turns
        .as_array()
        .and_then(|items| {
            turn_id
                .and_then(|id| {
                    items
                        .iter()
                        .find(|item| item.get("id").and_then(Value::as_str) == Some(id))
                })
                .or_else(|| items.last())
        })
        .cloned()
        .unwrap_or(Value::Null);
    (metadata, turn)
}

fn write_new_json(path: &Path, value: &impl Serialize) -> AppResult<()> {
    if path.exists() {
        return Err(AppError::Message(format!(
            "provider checkpoint already exists: {}",
            path.display()
        )));
    }
    let text = serde_json::to_string_pretty(value)?;
    std::fs::write(path, format!("{text}\n"))?;
    Ok(())
}

fn read_checkpoint_path(path: &Path) -> AppResult<ProviderCheckpoint> {
    let raw = std::fs::read_to_string(path)?;
    let record: ProviderCheckpoint = serde_json::from_str(&raw)?;
    if record.version != CHECKPOINT_VERSION {
        return Err(AppError::Message(format!(
            "unsupported provider checkpoint version {}",
            record.version
        )));
    }
    Ok(record)
}

fn summary(record: &ProviderCheckpoint) -> ProviderCheckpointSummary {
    ProviderCheckpointSummary {
        checkpoint_id: record.checkpoint_id.clone(),
        provider_id: record.provider_id.clone(),
        mnelyra_session_id: record.mnelyra_session_id.clone(),
        provider_session_id: record.provider_session_id.clone(),
        provider_turn_id: record.provider_turn_id.clone(),
        captured_at: record.captured_at.clone(),
        content_sha256: record.content_sha256.clone(),
    }
}

fn validate_checkpoint_id(value: &str) -> AppResult<()> {
    if value.len() > 80
        || value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(AppError::Message("invalid provider checkpoint id".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoint_id_rejects_path_traversal() {
        assert!(validate_checkpoint_id("../outside").is_err());
        assert!(validate_checkpoint_id("safe-id_123").is_ok());
    }

    #[test]
    fn snapshot_keeps_only_selected_turn_plus_thread_metadata() {
        let source = serde_json::json!({
            "thread": {
                "id": "thr_1",
                "name": "demo",
                "turns": [
                    { "id": "turn_1", "status": "completed" },
                    { "id": "turn_2", "status": "completed" }
                ]
            }
        });
        let (metadata, turn) = split_thread_snapshot(&source, Some("turn_1"));
        assert_eq!(metadata["id"], "thr_1");
        assert!(metadata.get("turns").is_none());
        assert_eq!(turn["id"], "turn_1");
    }
}
