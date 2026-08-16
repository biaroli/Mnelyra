use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tokio::sync::Mutex as AsyncMutex;

use crate::activity::{ActivityCoordinator, ActivityGuard, ActivityKind};
use crate::error::{AppError, AppResult};
use crate::provider::{AppServerEvent, CodexAppServerManager};

use super::events::timestamp;
use super::memory::write_provider_checkpoint;
use super::{
    PendingSessionRequest, SessionEvent, SessionEventPage, SessionEventStore, TaskSession,
    TaskSessionState, TaskSessionStore,
};

struct CodexBinding {
    thread_id: String,
    active_turn_id: Option<String>,
    turn_guard: Option<ActivityGuard>,
    manual_compaction: bool,
}

#[derive(Debug, Clone)]
struct PendingInteraction {
    public: PendingSessionRequest,
    wire_id: Value,
}

#[derive(Clone)]
pub struct SessionCoordinator {
    app_server: CodexAppServerManager,
    sessions: TaskSessionStore,
    events: SessionEventStore,
    activity: ActivityCoordinator,
    bindings: Arc<AsyncMutex<HashMap<String, CodexBinding>>>,
    pending: Arc<Mutex<HashMap<String, PendingInteraction>>>,
    monitor_started: Arc<AtomicBool>,
}

impl SessionCoordinator {
    pub fn list_sessions(&self) -> AppResult<Vec<TaskSession>> {
        self.sessions.list()
    }

    pub fn get_session(&self, session_id: &str) -> AppResult<TaskSession> {
        self.sessions.get(session_id)
    }

    pub async fn compact(&self, session_id: &str) -> AppResult<TaskSession> {
        self.ensure_monitor().await.map_err(AppError::Message)?;
        self.ensure_binding(session_id).await?;
        let session = self.sessions.get(session_id)?;
        let thread_id = {
            let mut bindings = self.bindings.lock().await;
            let binding = bindings.get_mut(session_id).ok_or_else(|| {
                AppError::Message(format!("session {session_id} has no provider binding"))
            })?;
            if binding.active_turn_id.is_some() || binding.turn_guard.is_some() {
                return Err(AppError::Message(format!(
                    "session {session_id} is busy; compact only when no turn is active"
                )));
            }
            let guard = self
                .activity
                .acquire(&session.workspace_id, ActivityKind::ProviderTurn)
                .map_err(|error| AppError::Message(error.to_string()))?;
            binding.turn_guard = Some(guard);
            binding.manual_compaction = true;
            binding.thread_id.clone()
        };
        self.sessions
            .update_state(session_id, TaskSessionState::Compacting)?;
        self.events.append(
            session_id,
            "compaction_requested",
            "Manual Codex context compaction requested",
            None,
        )?;
        if let Err(error) = self.app_server.compact_thread(&thread_id).await {
            self.release_turn(session_id).await;
            let _ = self
                .sessions
                .update_state(session_id, TaskSessionState::WaitingForUser);
            let _ = self.events.append(session_id, "error", &error, None);
            return Err(AppError::Message(error));
        }
        self.sessions.get(session_id)
    }

    pub fn new(
        app_server: CodexAppServerManager,
        sessions: TaskSessionStore,
        events: SessionEventStore,
        activity: ActivityCoordinator,
    ) -> Self {
        Self {
            app_server,
            sessions,
            events,
            activity,
            bindings: Arc::new(AsyncMutex::new(HashMap::new())),
            pending: Arc::new(Mutex::new(HashMap::new())),
            monitor_started: Arc::new(AtomicBool::new(false)),
        }
    }

    async fn ensure_binding(&self, session_id: &str) -> AppResult<()> {
        if self.bindings.lock().await.contains_key(session_id) {
            return Ok(());
        }
        let session = self.sessions.get(session_id)?;
        let thread_id = session.provider_session_id.clone().ok_or_else(|| {
            AppError::Message(format!(
                "session {session_id} has no provider thread binding"
            ))
        })?;
        self.app_server
            .resume_thread(&thread_id)
            .await
            .map_err(AppError::Message)?;
        self.bindings.lock().await.insert(
            session_id.to_string(),
            CodexBinding {
                thread_id: thread_id.clone(),
                active_turn_id: None,
                turn_guard: None,
                manual_compaction: false,
            },
        );
        self.events.append(
            session_id,
            "thread_resumed",
            format!("Recovered Codex thread {thread_id}"),
            None,
        )?;
        Ok(())
    }

    pub async fn start_codex_task(
        &self,
        workspace_id: &str,
        workspace_root: &Path,
        title: &str,
        prompt: &str,
    ) -> AppResult<TaskSession> {
        let canonical = std::fs::canonicalize(workspace_root).map_err(|error| {
            AppError::Message(format!(
                "failed to canonicalize workspace {}: {error}",
                workspace_root.display()
            ))
        })?;
        let title = normalized_title(title, prompt);
        let session =
            self.sessions
                .create(workspace_id, canonical.to_string_lossy(), "codex", title)?;
        self.sessions
            .update_state(&session.id, TaskSessionState::Starting)?;
        self.events.append(
            &session.id,
            "system",
            "Starting Codex app-server thread",
            None,
        )?;

        let _operation = self
            .activity
            .acquire(workspace_id, ActivityKind::ProviderOperation)
            .map_err(|error| AppError::Message(error.to_string()))?;

        if let Err(error) = self.ensure_monitor().await {
            let _ = self
                .sessions
                .update_state(&session.id, TaskSessionState::Failed);
            let _ = self.events.append(&session.id, "error", &error, None);
            return Err(AppError::Message(error));
        }

        let thread_id = match self.app_server.start_thread(&canonical).await {
            Ok(thread_id) => thread_id,
            Err(error) => {
                let _ = self
                    .sessions
                    .update_state(&session.id, TaskSessionState::Failed);
                let _ = self.events.append(&session.id, "error", &error, None);
                return Err(AppError::Message(error));
            }
        };
        self.sessions
            .bind_provider_session(&session.id, &thread_id)?;
        self.bindings.lock().await.insert(
            session.id.clone(),
            CodexBinding {
                thread_id: thread_id.clone(),
                active_turn_id: None,
                turn_guard: None,
                manual_compaction: false,
            },
        );
        self.events.append(
            &session.id,
            "thread_started",
            format!("Codex thread {thread_id} bound to workspace"),
            None,
        )?;

        if prompt.trim().is_empty() {
            return self
                .sessions
                .update_state(&session.id, TaskSessionState::WaitingForUser);
        }

        self.start_turn_internal(&session.id, prompt).await
    }

    pub async fn send_input(&self, session_id: &str, input: &str) -> AppResult<TaskSession> {
        let session = self.sessions.get(session_id)?;
        if matches!(
            session.state,
            TaskSessionState::Completed | TaskSessionState::Cancelled | TaskSessionState::Failed
        ) {
            return Err(AppError::Message(format!(
                "session {session_id} is terminal ({:?})",
                session.state
            )));
        }
        self.start_turn_internal(session_id, input).await
    }

    pub async fn cancel(&self, session_id: &str) -> AppResult<TaskSession> {
        let session = self.sessions.get(session_id)?;
        let (thread_id, turn_id) = {
            let bindings = self.bindings.lock().await;
            let binding = bindings.get(session_id).ok_or_else(|| {
                AppError::Message(format!("session {session_id} has no provider binding"))
            })?;
            let turn_id = binding.active_turn_id.clone().ok_or_else(|| {
                AppError::Message(format!("session {session_id} has no active turn"))
            })?;
            (binding.thread_id.clone(), turn_id)
        };

        let _operation = self
            .activity
            .acquire(&session.workspace_id, ActivityKind::ProviderOperation)
            .map_err(|error| AppError::Message(error.to_string()))?;
        self.sessions
            .update_state(session_id, TaskSessionState::Draining)?;
        self.events.append(
            session_id,
            "system",
            format!("Interrupt requested for turn {turn_id}"),
            None,
        )?;
        if let Err(error) = self.app_server.interrupt_turn(&thread_id, &turn_id).await {
            let _ = self
                .sessions
                .update_state(session_id, TaskSessionState::Failed);
            let _ = self.events.append(session_id, "error", &error, None);
            self.release_turn(session_id).await;
            return Err(AppError::Message(error));
        }
        self.sessions.get(session_id)
    }

    pub fn events(&self, session_id: &str) -> AppResult<Vec<SessionEvent>> {
        self.events.list(session_id)
    }

    pub fn event_page(
        &self,
        session_id: &str,
        cursor: u64,
        limit: Option<usize>,
    ) -> AppResult<SessionEventPage> {
        self.events.page(session_id, cursor, limit)
    }

    pub fn pending_requests(&self, session_id: &str) -> AppResult<Vec<PendingSessionRequest>> {
        let pending = self
            .pending
            .lock()
            .map_err(|_| AppError::Message("pending session request store poisoned".into()))?;
        let mut requests = pending
            .values()
            .filter(|request| request.public.session_id == session_id)
            .map(|request| request.public.clone())
            .collect::<Vec<_>>();
        requests.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok(requests)
    }

    pub async fn respond_to_request(
        &self,
        session_id: &str,
        request_id: &str,
        action: &str,
    ) -> AppResult<()> {
        let request = {
            let pending = self
                .pending
                .lock()
                .map_err(|_| AppError::Message("pending session request store poisoned".into()))?;
            let request = pending.get(request_id).cloned().ok_or_else(|| {
                AppError::Message(format!("pending session request not found: {request_id}"))
            })?;
            if request.public.session_id != session_id {
                return Err(AppError::Message(
                    "pending request belongs to another session".into(),
                ));
            }
            request
        };

        let result = approval_result(&request.public.method, &request.public.params, action)?;
        self.app_server
            .respond_to_server_request(request.wire_id.clone(), result)
            .await
            .map_err(AppError::Message)?;
        self.pending
            .lock()
            .map_err(|_| AppError::Message("pending session request store poisoned".into()))?
            .remove(request_id);
        let _ = self
            .sessions
            .update_state(session_id, TaskSessionState::Running);
        self.events.append(
            session_id,
            "approval",
            format!("{} → {action}", request.public.method),
            None,
        )?;
        Ok(())
    }

    async fn start_turn_internal(&self, session_id: &str, input: &str) -> AppResult<TaskSession> {
        if input.trim().is_empty() {
            return Err(AppError::Message("session input must not be empty".into()));
        }
        self.ensure_monitor().await.map_err(AppError::Message)?;
        let session = self.sessions.get(session_id)?;
        let _operation = self
            .activity
            .acquire(&session.workspace_id, ActivityKind::ProviderOperation)
            .map_err(|error| AppError::Message(error.to_string()))?;
        self.ensure_binding(session_id).await?;
        let thread_id = {
            let mut bindings = self.bindings.lock().await;
            let binding = bindings.get_mut(session_id).ok_or_else(|| {
                AppError::Message(format!("session {session_id} has no provider binding"))
            })?;
            if binding.active_turn_id.is_some() || binding.turn_guard.is_some() {
                return Err(AppError::Message(format!(
                    "session {session_id} already has an active turn"
                )));
            }
            let guard = self
                .activity
                .acquire(&session.workspace_id, ActivityKind::ProviderTurn)
                .map_err(|error| AppError::Message(error.to_string()))?;
            binding.turn_guard = Some(guard);
            binding.thread_id.clone()
        };
        self.sessions
            .update_state(session_id, TaskSessionState::Starting)?;
        self.events
            .append(session_id, "user", input.to_string(), None)?;

        match self.app_server.start_turn(&thread_id, input).await {
            Ok(turn_id) => {
                let mut bindings = self.bindings.lock().await;
                if let Some(binding) = bindings.get_mut(session_id) {
                    binding.active_turn_id = Some(turn_id.clone());
                }
                drop(bindings);
                self.events.append(
                    session_id,
                    "turn_started",
                    format!("Codex turn {turn_id} started"),
                    None,
                )?;
                self.sessions
                    .update_state(session_id, TaskSessionState::Running)
            }
            Err(error) => {
                self.release_turn(session_id).await;
                let _ = self
                    .sessions
                    .update_state(session_id, TaskSessionState::Failed);
                let _ = self.events.append(session_id, "error", &error, None);
                Err(AppError::Message(error))
            }
        }
    }

    async fn ensure_monitor(&self) -> Result<(), String> {
        if self
            .monitor_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Ok(());
        }

        let receiver = match self.app_server.subscribe().await {
            Ok(receiver) => receiver,
            Err(error) => {
                self.monitor_started.store(false, Ordering::Release);
                return Err(error);
            }
        };
        let coordinator = self.clone();
        tauri::async_runtime::spawn(async move {
            coordinator.monitor_loop(receiver).await;
            coordinator.monitor_started.store(false, Ordering::Release);
        });
        Ok(())
    }

    async fn monitor_loop(&self, mut receiver: tokio::sync::broadcast::Receiver<AppServerEvent>) {
        loop {
            match receiver.recv().await {
                Ok(event) => self.handle_event(event).await,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    for session in self.sessions.list().unwrap_or_default() {
                        if self.bindings.lock().await.contains_key(&session.id) {
                            let _ = self.events.append(
                                &session.id,
                                "warning",
                                format!("Codex event stream lagged; {skipped} event(s) skipped"),
                                None,
                            );
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
        self.fail_active_sessions("Codex app-server event stream closed")
            .await;
    }

    async fn handle_event(&self, event: AppServerEvent) {
        let payload = event.payload;
        let method = payload
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if method.is_empty() {
            return;
        }

        let params = payload.get("params").cloned().unwrap_or(Value::Null);
        let thread_id = first_string(
            &params,
            &[&["threadId"], &["thread", "id"], &["turn", "threadId"]],
        );
        let turn_id = first_string(&params, &[&["turnId"], &["turn", "id"]]);
        let session_id = self
            .find_session(thread_id.as_deref(), turn_id.as_deref())
            .await;

        if payload.get("id").is_some() {
            if let Some(session_id) = session_id {
                self.record_server_request(&session_id, method, params, payload["id"].clone())
                    .await;
            }
            return;
        }

        let Some(session_id) = session_id else {
            return;
        };
        match method {
            "thread/tokenUsage/updated" => {
                let total_tokens = params
                    .pointer("/tokenUsage/total/totalTokens")
                    .and_then(Value::as_u64);
                let context_window = params.get("modelContextWindow").and_then(Value::as_u64);
                let message = match (total_tokens, context_window) {
                    (Some(total), Some(window)) => {
                        format!("Context usage {total} / {window} tokens")
                    }
                    (Some(total), None) => format!("Context usage {total} tokens"),
                    _ => "Codex context usage updated".into(),
                };
                let _ =
                    self.events
                        .append(&session_id, "token_usage", message, Some(params.clone()));
            }
            "turn/started" => {
                let _ = self
                    .sessions
                    .update_state(&session_id, TaskSessionState::Running);
            }
            "turn/completed" => {
                let status = first_string(&params, &[&["turn", "status"], &["status"]])
                    .unwrap_or_else(|| "completed".into());
                let current = self.sessions.get(&session_id).ok();
                let next = if current
                    .as_ref()
                    .is_some_and(|session| session.state == TaskSessionState::Draining)
                    || status.eq_ignore_ascii_case("interrupted")
                {
                    TaskSessionState::Cancelled
                } else if status.eq_ignore_ascii_case("failed") {
                    TaskSessionState::Failed
                } else {
                    TaskSessionState::WaitingForUser
                };
                let _ = self.sessions.update_state(&session_id, next);
                let _ = self.events.append(
                    &session_id,
                    "turn_completed",
                    format!("Turn completed with status {status}"),
                    None,
                );
                self.release_turn(&session_id).await;
                self.clear_pending_for_session(&session_id);
                self.schedule_provider_checkpoint(&session_id, turn_id.as_deref());
            }
            "item/agentMessage/delta" => {
                if let Some(delta) = first_string(&params, &[&["delta"], &["text"]]) {
                    let _ = self
                        .events
                        .append(&session_id, "assistant_delta", delta, None);
                }
            }
            "item/started" => {
                let item_type =
                    first_string(&params, &[&["item", "type"]]).unwrap_or_else(|| "item".into());
                if item_type == "contextCompaction" {
                    let _ = self
                        .sessions
                        .update_state(&session_id, TaskSessionState::Compacting);
                    let _ = self.events.append(
                        &session_id,
                        "compaction_started",
                        "Codex context compaction started",
                        Some(params.clone()),
                    );
                }
            }
            "item/completed" => {
                let item_type =
                    first_string(&params, &[&["item", "type"]]).unwrap_or_else(|| "item".into());
                if item_type == "contextCompaction" {
                    let manual = {
                        let bindings = self.bindings.lock().await;
                        bindings
                            .get(&session_id)
                            .is_some_and(|binding| binding.manual_compaction)
                    };
                    let _ = self.sessions.update_state(
                        &session_id,
                        if manual {
                            TaskSessionState::WaitingForUser
                        } else {
                            TaskSessionState::Running
                        },
                    );
                    let _ = self.events.append(
                        &session_id,
                        "compaction_completed",
                        "Codex context compaction completed",
                        Some(params.clone()),
                    );
                    if manual {
                        self.release_turn(&session_id).await;
                    }
                } else if item_type == "agentMessage" {
                    if let Some(text) = first_string(&params, &[&["item", "text"]]) {
                        if self
                            .events
                            .list(&session_id)
                            .unwrap_or_default()
                            .last()
                            .is_none_or(|event| event.kind != "assistant_delta")
                        {
                            let _ = self
                                .events
                                .append(&session_id, "assistant_delta", text, None);
                        }
                    }
                } else {
                    let status = first_string(&params, &[&["item", "status"]])
                        .unwrap_or_else(|| "completed".into());
                    let _ = self.events.append(
                        &session_id,
                        "tool",
                        format!("{item_type} · {status}"),
                        None,
                    );
                }
            }
            "serverRequest/resolved" => {
                if let Some(request_id) = params.get("requestId") {
                    self.remove_pending_by_wire_id(request_id);
                }
            }
            _ => {}
        }
    }

    async fn record_server_request(
        &self,
        session_id: &str,
        method: &str,
        params: Value,
        wire_id: Value,
    ) {
        let id = uuid::Uuid::new_v4().to_string();
        let public = PendingSessionRequest {
            id: id.clone(),
            session_id: session_id.to_string(),
            method: method.to_string(),
            params,
            created_at: timestamp(),
        };
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(
                id,
                PendingInteraction {
                    public: public.clone(),
                    wire_id,
                },
            );
        }
        let _ = self
            .sessions
            .update_state(session_id, TaskSessionState::WaitingForUser);
        let _ = self.events.append(
            session_id,
            "approval_required",
            format!("Codex requires client response: {method}"),
            Some(public.params.clone()),
        );
    }

    async fn find_session(&self, thread_id: Option<&str>, turn_id: Option<&str>) -> Option<String> {
        let bindings = self.bindings.lock().await;
        bindings.iter().find_map(|(session_id, binding)| {
            let thread_match = thread_id.is_some_and(|id| binding.thread_id == id);
            let turn_match =
                turn_id.is_some_and(|id| binding.active_turn_id.as_deref() == Some(id));
            (thread_match || turn_match).then(|| session_id.clone())
        })
    }

    async fn release_turn(&self, session_id: &str) {
        let mut bindings = self.bindings.lock().await;
        if let Some(binding) = bindings.get_mut(session_id) {
            binding.active_turn_id = None;
            binding.turn_guard.take();
            binding.manual_compaction = false;
        }
    }

    async fn fail_active_sessions(&self, message: &str) {
        let session_ids = {
            let mut bindings = self.bindings.lock().await;
            bindings
                .iter_mut()
                .filter_map(|(id, binding)| {
                    let active =
                        binding.active_turn_id.take().is_some() || binding.turn_guard.is_some();
                    binding.turn_guard.take();
                    active.then(|| id.clone())
                })
                .collect::<Vec<_>>()
        };
        for id in session_ids {
            let _ = self.sessions.update_state(&id, TaskSessionState::Failed);
            let _ = self.events.append(&id, "error", message, None);
            self.clear_pending_for_session(&id);
        }
    }

    fn clear_pending_for_session(&self, session_id: &str) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.retain(|_, request| request.public.session_id != session_id);
        }
    }

    fn remove_pending_by_wire_id(&self, wire_id: &Value) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.retain(|_, request| &request.wire_id != wire_id);
        }
    }

    fn schedule_provider_checkpoint(&self, session_id: &str, turn_id: Option<&str>) {
        let Ok(session) = self.sessions.get(session_id) else {
            return;
        };
        let Some(thread_id) = session.provider_session_id.clone() else {
            return;
        };
        let turn_id = turn_id.map(str::to_string);
        let app_server = self.app_server.clone();
        let events = self.events.clone();
        tauri::async_runtime::spawn(async move {
            match app_server.read_thread(&thread_id).await {
                Ok(thread_read) => {
                    match write_provider_checkpoint(&session, turn_id.as_deref(), &thread_read) {
                        Ok(checkpoint) => {
                            let _ = events.append(
                                &session.id,
                                "memory_checkpoint",
                                format!("Provider checkpoint {} written", checkpoint.checkpoint_id),
                                Some(json!({
                                    "checkpointId": checkpoint.checkpoint_id,
                                    "contentSha256": checkpoint.content_sha256,
                                    "source": checkpoint.source,
                                })),
                            );
                        }
                        Err(error) => {
                            let _ = events.append(
                                &session.id,
                                "warning",
                                format!("Provider checkpoint write failed: {error}"),
                                None,
                            );
                        }
                    }
                }
                Err(error) => {
                    let _ = events.append(
                        &session.id,
                        "warning",
                        format!("Provider checkpoint capture failed: {error}"),
                        None,
                    );
                }
            }
        });
    }
}

fn normalized_title(title: &str, prompt: &str) -> String {
    let title = title.trim();
    if !title.is_empty() {
        return title.chars().take(96).collect();
    }
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return "Codex task".into();
    }
    prompt.chars().take(64).collect()
}

fn first_string(value: &Value, paths: &[&[&str]]) -> Option<String> {
    paths.iter().find_map(|path| {
        let mut current = value;
        for part in *path {
            current = current.get(*part)?;
        }
        current.as_str().map(str::to_string)
    })
}

fn approval_result(method: &str, params: &Value, action: &str) -> AppResult<Value> {
    let normalized = match action {
        "accept" => "accept",
        "accept_for_session" => "acceptForSession",
        "decline" => "decline",
        "cancel" => "cancel",
        _ => {
            return Err(AppError::Message(format!(
                "unsupported approval action: {action}"
            )))
        }
    };

    if method == "item/commandExecution/requestApproval"
        || method == "item/fileChange/requestApproval"
        || method == "execCommandApproval"
        || method == "applyPatchApproval"
    {
        return Ok(json!({ "decision": normalized }));
    }

    if method == "item/permissions/requestApproval" {
        if normalized == "accept" || normalized == "acceptForSession" {
            let requested = params
                .get("permissions")
                .cloned()
                .unwrap_or_else(|| json!({}));
            return Ok(json!({
                "permissions": requested,
                "scope": if normalized == "acceptForSession" { "session" } else { "turn" }
            }));
        }
        return Ok(json!({ "permissions": {} }));
    }

    if method == "mcpServer/elicitation/request"
        && (normalized == "decline" || normalized == "cancel")
    {
        return Ok(json!({ "action": normalized, "content": null }));
    }

    Err(AppError::Message(format!(
        "Mnelyra does not yet have a safe response renderer for {method}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_accept_returns_exact_requested_subset_only() {
        let params = json!({
            "permissions": {
                "fileSystem": { "write": ["E:\\project\\tex-cache"] }
            }
        });
        let result = approval_result("item/permissions/requestApproval", &params, "accept")
            .expect("approval result");
        assert_eq!(result["permissions"], params["permissions"]);
        assert_eq!(result["scope"], "turn");
    }

    #[test]
    fn unsupported_interaction_is_not_auto_approved() {
        assert!(approval_result("item/tool/requestUserInput", &json!({}), "accept").is_err());
    }
}
