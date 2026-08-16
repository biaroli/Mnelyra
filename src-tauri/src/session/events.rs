use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{AppError, AppResult};

const MAX_EVENTS_PER_SESSION: usize = 600;
const MAX_EVENT_TEXT_CHARS: usize = 16 * 1024;
const DEFAULT_EVENT_PAGE_LIMIT: usize = 160;
const MAX_EVENT_PAGE_LIMIT: usize = 240;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionEvent {
    pub id: String,
    pub session_id: String,
    pub kind: String,
    pub text: String,
    pub created_at: String,
    pub details: Option<Value>,
    pub revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionEventPage {
    pub events: Vec<SessionEvent>,
    pub next_cursor: u64,
    pub reset: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PendingSessionRequest {
    pub id: String,
    pub session_id: String,
    pub method: String,
    pub params: Value,
    pub created_at: String,
}

#[derive(Default)]
struct SessionEventLog {
    revision: u64,
    events: VecDeque<SessionEvent>,
}

#[derive(Clone, Default)]
pub struct SessionEventStore {
    inner: Arc<Mutex<BTreeMap<String, SessionEventLog>>>,
}

impl SessionEventStore {
    pub fn append(
        &self,
        session_id: &str,
        kind: impl Into<String>,
        text: impl Into<String>,
        details: Option<Value>,
    ) -> AppResult<SessionEvent> {
        let kind = kind.into();
        let text = bounded_text(text.into());
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| AppError::Message("session event store poisoned".into()))?;
        let log = inner.entry(session_id.to_string()).or_default();
        log.revision = log.revision.saturating_add(1);
        let revision = log.revision;

        if kind == "assistant_delta" {
            if let Some(last) = log.events.back_mut() {
                if last.kind == kind && last.text.chars().count() < MAX_EVENT_TEXT_CHARS {
                    last.text = bounded_text(format!("{}{}", last.text, text));
                    last.created_at = timestamp();
                    last.revision = revision;
                    return Ok(last.clone());
                }
            }
        }

        let event = SessionEvent {
            id: uuid::Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            kind,
            text,
            created_at: timestamp(),
            details,
            revision,
        };
        log.events.push_back(event.clone());
        while log.events.len() > MAX_EVENTS_PER_SESSION {
            log.events.pop_front();
        }
        Ok(event)
    }

    pub fn list(&self, session_id: &str) -> AppResult<Vec<SessionEvent>> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| AppError::Message("session event store poisoned".into()))?;
        Ok(inner
            .get(session_id)
            .map(|log| log.events.iter().cloned().collect())
            .unwrap_or_default())
    }

    pub fn page(
        &self,
        session_id: &str,
        cursor: u64,
        limit: Option<usize>,
    ) -> AppResult<SessionEventPage> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| AppError::Message("session event store poisoned".into()))?;
        let Some(log) = inner.get(session_id) else {
            return Ok(SessionEventPage {
                events: Vec::new(),
                next_cursor: 0,
                reset: cursor > 0,
            });
        };
        let limit = limit
            .unwrap_or(DEFAULT_EVENT_PAGE_LIMIT)
            .clamp(1, MAX_EVENT_PAGE_LIMIT);
        let oldest_revision = log.events.front().map(|event| event.revision).unwrap_or(0);
        let cursor_fell_behind = cursor > 0 && oldest_revision > cursor.saturating_add(1);
        let changed_count = log
            .events
            .iter()
            .filter(|event| event.revision > cursor)
            .count();
        let reset = cursor_fell_behind || changed_count > limit;

        let events = if cursor == 0 || reset {
            let mut recent = log
                .events
                .iter()
                .rev()
                .take(limit)
                .cloned()
                .collect::<Vec<_>>();
            recent.reverse();
            recent
        } else {
            log.events
                .iter()
                .filter(|event| event.revision > cursor)
                .cloned()
                .collect::<Vec<_>>()
        };

        Ok(SessionEventPage {
            events,
            next_cursor: log.revision,
            reset,
        })
    }
}

pub(crate) fn timestamp() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .to_string()
}

fn bounded_text(value: String) -> String {
    let mut chars = value.chars();
    let bounded = chars
        .by_ref()
        .take(MAX_EVENT_TEXT_CHARS)
        .collect::<String>();
    if chars.next().is_some() {
        format!("{bounded}…[truncated]")
    } else {
        bounded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assistant_deltas_are_coalesced() {
        let store = SessionEventStore::default();
        store
            .append("s1", "assistant_delta", "hello", None)
            .expect("append");
        store
            .append("s1", "assistant_delta", " world", None)
            .expect("append");
        let events = store.list("s1").expect("list");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].text, "hello world");
        assert_eq!(events[0].revision, 2);
    }

    #[test]
    fn event_page_returns_only_revisions_after_cursor() {
        let store = SessionEventStore::default();
        store.append("s1", "system", "one", None).expect("append");
        let first = store.page("s1", 0, Some(10)).expect("first page");
        assert_eq!(first.events.len(), 1);
        assert_eq!(first.next_cursor, 1);

        store
            .append("s1", "assistant_delta", "hello", None)
            .expect("append");
        store
            .append("s1", "assistant_delta", " world", None)
            .expect("append");
        let delta = store
            .page("s1", first.next_cursor, Some(10))
            .expect("delta page");
        assert!(!delta.reset);
        assert_eq!(delta.events.len(), 1);
        assert_eq!(delta.events[0].text, "hello world");
        assert_eq!(delta.next_cursor, 3);
    }
}
