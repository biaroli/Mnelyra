use std::convert::Infallible;
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::body::{Body, Bytes};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::Response;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, WebviewWindow};
use tokio::sync::{mpsc, Mutex};
use tokio_stream::wrappers::ReceiverStream;

use crate::error::{AppError, AppResult};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(2);
const TURN_TIMEOUT: Duration = Duration::from_secs(2 * 60 * 60);
const STREAM_POLL_INTERVAL: Duration = Duration::from_millis(100);
const STREAM_RENDER_INTERVAL: Duration = Duration::from_millis(200);
const STREAM_TEXT_TAIL_HOLD_CHARS: usize = 48;
const COMPLETION_STABLE_POLLS: u8 = 3;
const UI_SETTLE: Duration = Duration::from_millis(250);
const INSERT_CHUNK_CHARS: usize = 16_000;
const MAX_PROMPT_CHARS: usize = 480_000;

static TURN_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn turn_lock() -> &'static Mutex<()> {
    TURN_LOCK.get_or_init(|| Mutex::new(()))
}

fn common_prefix_byte_len(left: &str, right: &str) -> usize {
    let mut bytes = 0usize;
    for (left_char, right_char) in left.chars().zip(right.chars()) {
        if left_char != right_char {
            break;
        }
        bytes += left_char.len_utf8();
    }
    bytes
}

fn stable_stream_end(text: &str, common_prefix_bytes: usize, hold_chars: usize) -> usize {
    let common_prefix_bytes = common_prefix_bytes.min(text.len());
    let common_prefix_bytes = if text.is_char_boundary(common_prefix_bytes) {
        common_prefix_bytes
    } else {
        text.char_indices()
            .map(|(index, _)| index)
            .take_while(|index| *index < common_prefix_bytes)
            .last()
            .unwrap_or(0)
    };
    let prefix = &text[..common_prefix_bytes];
    if hold_chars == 0 {
        return common_prefix_bytes;
    }
    let prefix_chars = prefix.chars().count();
    if prefix_chars <= hold_chars {
        return 0;
    }
    let keep_chars = prefix_chars - hold_chars;
    prefix
        .char_indices()
        .nth(keep_chars)
        .map(|(index, _)| index)
        .unwrap_or(common_prefix_bytes)
}

fn reconcile_streamed_with_final(emitted: &str, final_markdown: &str) -> Option<String> {
    if emitted.is_empty() {
        return Some(final_markdown.to_string());
    }
    if final_markdown.starts_with(emitted) {
        return Some(final_markdown.to_string());
    }

    let emitted_chars: Vec<(usize, char)> = emitted.char_indices().collect();
    let max_anchor_chars = emitted_chars.len().min(64);
    let min_anchor_chars = 24.min(max_anchor_chars);
    if min_anchor_chars == 0 {
        return None;
    }

    for anchor_chars in (min_anchor_chars..=max_anchor_chars).rev() {
        let start_char = emitted_chars.len() - anchor_chars;
        let start_byte = emitted_chars[start_char].0;
        let anchor = &emitted[start_byte..];
        let mut matches = final_markdown.match_indices(anchor);
        let Some((position, _)) = matches.next() else {
            continue;
        };
        if matches.next().is_some() {
            continue;
        }
        let suffix_start = position + anchor.len();
        let mut reconciled =
            String::with_capacity(emitted.len() + final_markdown.len() - suffix_start);
        reconciled.push_str(emitted);
        reconciled.push_str(&final_markdown[suffix_start..]);
        return Some(reconciled);
    }
    None
}

fn normalize_rendered_blocks(blocks: &[String]) -> String {
    let joined = blocks.concat();
    let mut normalized = String::with_capacity(joined.len());
    let mut newline_run = 0usize;
    for ch in joined.chars() {
        if ch == '\n' {
            newline_run += 1;
            if newline_run <= 2 {
                normalized.push(ch);
            }
        } else {
            newline_run = 0;
            normalized.push(ch);
        }
    }
    normalized.trim().to_string()
}

fn first_difference_char(left: &str, right: &str) -> Option<usize> {
    let mut left_chars = left.chars();
    let mut right_chars = right.chars();
    let mut index = 0usize;
    loop {
        match (left_chars.next(), right_chars.next()) {
            (Some(a), Some(b)) if a == b => index += 1,
            (Some(_), Some(_)) | (Some(_), None) | (None, Some(_)) => return Some(index),
            (None, None) => return None,
        }
    }
}

struct BusyFlag;

impl BusyFlag {
    fn set() -> Self {
        super::BROWSER_BUSY.store(true, std::sync::atomic::Ordering::Release);
        Self
    }
}

impl Drop for BusyFlag {
    fn drop(&mut self) {
        super::BROWSER_BUSY.store(false, std::sync::atomic::Ordering::Release);
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EffortSurface {
    available: bool,
    kind: Option<String>,
    min: Option<i64>,
    max: Option<i64>,
    value: Option<i64>,
    x: Option<f64>,
    y: Option<f64>,
    diagnostics: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ComposerState {
    text: String,
    inner_text_len: u64,
    user_turn_count: u64,
    assistant_turn_count: u64,
    send_ready: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TurnState {
    markdown: String,
    blocks: Vec<String>,
    streamable_count: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TurnProbe {
    assistant_present: bool,
    running: bool,
    completion_action_visible: bool,
    content_signature: String,
    content_text_len: u64,
    error: Option<String>,
    user_turn_count: u64,
    assistant_turn_count: u64,
}

pub(super) async fn responses(app: &AppHandle, request: Value) -> Response {
    let model = request
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("mnelyra-web/unknown")
        .to_string();
    let response_id = format!("resp_{}", uuid::Uuid::new_v4().simple());
    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0);

    let (tx, rx) = mpsc::channel::<Result<Bytes, Infallible>>(64);
    let app = app.clone();
    let response_id_task = response_id.clone();
    let model_task = model.clone();
    tokio::spawn(async move {
        let stream_started = Instant::now();
        let mut sequence = 0_u64;
        if !send_event(
            &tx,
            "response.created",
            json!({
                "response": response_snapshot(
                    &response_id_task,
                    created_at,
                    "in_progress",
                    &model_task,
                    Vec::<Value>::new(),
                    None,
                )
            }),
            &mut sequence,
        )
        .await
        {
            return;
        }

        let item_id = format!("msg_{}", uuid::Uuid::new_v4().simple());
        let (delta_tx, mut delta_rx) = mpsc::channel::<String>(64);
        let mut turn = Box::pin(run_browser_turn(&app, &request, delta_tx));
        let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut output_started = false;
        let mut delta_open = true;
        let mut delta_count = 0_u64;
        let mut streamed_markdown = String::new();
        let result = loop {
            tokio::select! {
                result = &mut turn => break result,
                maybe_delta = delta_rx.recv(), if delta_open => {
                    match maybe_delta {
                        Some(delta) if !delta.is_empty() => {
                            if !output_started {
                                if !send_output_started(&tx, &item_id, &mut sequence).await {
                                    return;
                                }
                                output_started = true;
                            }
                            if !send_output_delta(&tx, &item_id, &delta, &mut sequence).await {
                                return;
                            }
                            delta_count = delta_count.saturating_add(1);
                            if delta_count <= 12 {
                                eprintln!(
                                    "[web-models stream] model={} delta={} elapsed_ms={} chars={}",
                                    model_task,
                                    delta_count,
                                    stream_started.elapsed().as_millis(),
                                    delta.chars().count()
                                );
                            }
                            streamed_markdown.push_str(&delta);
                        }
                        Some(_) => {}
                        None => delta_open = false,
                    }
                }
                _ = heartbeat.tick() => {
                    if !send_raw(&tx, "event: response.heartbeat\ndata: {\"type\":\"response.heartbeat\"}\n\n").await {
                        return;
                    }
                }
            }
        };

        match result {
            Ok(markdown) => {
                while let Ok(delta) = delta_rx.try_recv() {
                    if delta.is_empty() {
                        continue;
                    }
                    if !output_started && !send_output_started(&tx, &item_id, &mut sequence).await {
                        return;
                    }
                    if !send_output_delta(&tx, &item_id, &delta, &mut sequence).await {
                        return;
                    }
                    streamed_markdown.push_str(&delta);
                }

                if !markdown.starts_with(&streamed_markdown) {
                    let failed = response_snapshot(
                        &response_id_task,
                        created_at,
                        "failed",
                        &model_task,
                        Vec::<Value>::new(),
                        Some(
                            "Mnelyra browser stream diverged from the completed ChatGPT response"
                                .into(),
                        ),
                    );
                    let _ = send_event(
                        &tx,
                        "response.failed",
                        json!({"response": failed}),
                        &mut sequence,
                    )
                    .await;
                    let _ = send_raw(&tx, "data: [DONE]\n\n").await;
                    return;
                }

                let remainder = &markdown[streamed_markdown.len()..];
                if !remainder.is_empty() {
                    if !output_started && !send_output_started(&tx, &item_id, &mut sequence).await {
                        return;
                    }
                    if !send_output_delta(&tx, &item_id, remainder, &mut sequence).await {
                        return;
                    }
                    delta_count = delta_count.saturating_add(1);
                    if delta_count <= 12 {
                        eprintln!(
                            "[web-models stream] model={} delta={} elapsed_ms={} chars={} final_remainder=true",
                            model_task,
                            delta_count,
                            stream_started.elapsed().as_millis(),
                            remainder.chars().count()
                        );
                    }
                } else if !output_started
                    && !send_output_started(&tx, &item_id, &mut sequence).await
                {
                    return;
                }
                let _ = send_event(
                    &tx,
                    "response.output_text.done",
                    json!({
                        "item_id": item_id,
                        "output_index": 0,
                        "content_index": 0,
                        "text": markdown,
                    }),
                    &mut sequence,
                )
                .await;
                let part = json!({"type": "output_text", "text": markdown, "annotations": []});
                let _ = send_event(
                    &tx,
                    "response.content_part.done",
                    json!({
                        "item_id": item_id,
                        "output_index": 0,
                        "content_index": 0,
                        "part": part,
                    }),
                    &mut sequence,
                )
                .await;
                let item = json!({
                    "type": "message",
                    "id": item_id,
                    "status": "completed",
                    "role": "assistant",
                    "phase": "final_answer",
                    "content": [part],
                });
                let _ = send_event(
                    &tx,
                    "response.output_item.done",
                    json!({"output_index": 0, "item": item}),
                    &mut sequence,
                )
                .await;
                let completed = response_snapshot(
                    &response_id_task,
                    created_at,
                    "completed",
                    &model_task,
                    vec![item],
                    None,
                );
                let _ = send_event(
                    &tx,
                    "response.completed",
                    json!({"response": completed}),
                    &mut sequence,
                )
                .await;
                let _ = send_raw(&tx, "data: [DONE]\n\n").await;
                eprintln!(
                    "[web-models stream] model={} completed_ms={} deltas={}",
                    model_task,
                    stream_started.elapsed().as_millis(),
                    delta_count
                );
            }
            Err(error) => {
                let failed = response_snapshot(
                    &response_id_task,
                    created_at,
                    "failed",
                    &model_task,
                    Vec::<Value>::new(),
                    Some(error.to_string()),
                );
                let _ = send_event(
                    &tx,
                    "response.failed",
                    json!({"response": failed}),
                    &mut sequence,
                )
                .await;
                let _ = send_raw(&tx, "data: [DONE]\n\n").await;
            }
        }
    });

    let stream = ReceiverStream::new(rx);
    let mut response = Response::new(Body::from_stream(stream));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream; charset=utf-8"),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-store, must-revalidate"),
    );
    response
        .headers_mut()
        .insert(header::CONNECTION, HeaderValue::from_static("keep-alive"));
    response
}

async fn run_browser_turn(
    app: &AppHandle,
    request: &Value,
    delta_tx: mpsc::Sender<String>,
) -> AppResult<String> {
    let turn_started = Instant::now();
    let _turn = turn_lock().lock().await;
    let _busy = BusyFlag::set();
    let model = request
        .get("model")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::Message("Web-model Responses request is missing model".into()))?;
    let effort_index = effort_index(request)?;
    let prompt = compile_prompt(request)?;
    let prompt_chars = prompt.chars().count();
    if prompt_chars > MAX_PROMPT_CHARS {
        return Err(AppError::Message(format!(
            "This Codex context is too large for the current Mnelyra ChatGPT browser transport ({} characters > {MAX_PROMPT_CHARS}); compact the Codex thread and retry",
            prompt_chars
        )));
    }

    let window = super::ensure_browser_window(app)?;
    let phase_started = Instant::now();
    navigate_fresh_temporary_chat(&window, true).await?;
    eprintln!(
        "[web-models timing] model={model} phase=navigate_fresh_ms value={} prompt_chars={prompt_chars}",
        phase_started.elapsed().as_millis()
    );
    let phase_started = Instant::now();
    if let Err(first_error) = select_effort(&window, effort_index).await {
        // ChatGPT occasionally mounts the composer before the Radix effort
        // picker has finished settling. Recover locally before any prompt is
        // attached so Codex does not have to retry the entire 40K+ request.
        eprintln!(
            "[web-models] effort selection transient failure; refreshing fresh chat once: {first_error}"
        );
        let retry_started = Instant::now();
        navigate_fresh_temporary_chat(&window, false).await?;
        eprintln!(
            "[web-models timing] model={model} phase=effort_retry_navigate_ms value={}",
            retry_started.elapsed().as_millis()
        );
        select_effort(&window, effort_index)
            .await
            .map_err(|retry_error| {
                AppError::Message(format!(
                "ChatGPT effort selection failed twice; first: {first_error}; retry: {retry_error}"
            ))
            })?;
    }
    eprintln!(
        "[web-models timing] model={model} phase=select_effort_ms value={}",
        phase_started.elapsed().as_millis()
    );
    let phase_started = Instant::now();
    let initial = attach_prompt(&window, &prompt).await?;
    eprintln!(
        "[web-models timing] model={model} phase=attach_prompt_ms value={}",
        phase_started.elapsed().as_millis()
    );
    let phase_started = Instant::now();
    submit_prompt(&window, &initial).await?;
    eprintln!(
        "[web-models timing] model={model} phase=submit_ack_ms value={}",
        phase_started.elapsed().as_millis()
    );
    let result = stream_completed_markdown(&window, &delta_tx, turn_started).await;
    eprintln!(
        "[web-models timing] model={model} phase=turn_total_ms value={}",
        turn_started.elapsed().as_millis()
    );
    result
}

async fn navigate_fresh_temporary_chat(
    window: &WebviewWindow,
    allow_empty_page_reuse: bool,
) -> AppResult<()> {
    // start_browser_only already leaves an authenticated Temporary Chat open.
    // Reuse it for the first turn when it is provably empty instead of paying
    // for a second full ChatGPT navigation/hydration. Once a turn exists, or
    // when the effort-picker recovery path explicitly requests a refresh, keep
    // the old one-turn-per-page isolation by navigating to a fresh chat.
    if allow_empty_page_reuse && browser_turn_ready(window).await.unwrap_or(false) {
        if let Ok(probe) = read_turn_probe(window).await {
            if probe.user_turn_count == 0
                && probe.assistant_turn_count == 0
                && !probe.running
                && !probe.assistant_present
            {
                return Ok(());
            }
        }
    }

    let url = super::CHATGPT_TEMPORARY_URL.parse().map_err(|error| {
        AppError::Message(format!("invalid ChatGPT Temporary Chat URL: {error}"))
    })?;
    window.navigate(url).map_err(|error| {
        AppError::Message(format!("could not open a fresh Temporary Chat: {error}"))
    })?;
    super::wait_for_chatgpt_document(window).await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let auth_probe_at = tokio::time::Instant::now() + Duration::from_secs(2);
    let mut auth_checked = false;
    let mut stable_ready_polls = 0_u8;
    while tokio::time::Instant::now() < deadline {
        if browser_turn_ready(window).await.unwrap_or(false) {
            stable_ready_polls = stable_ready_polls.saturating_add(1);
            if stable_ready_polls >= 2 {
                return Ok(());
            }
        } else {
            stable_ready_polls = 0;
        }
        if !auth_checked && tokio::time::Instant::now() >= auth_probe_at {
            auth_checked = true;
            if let Ok(probe) = super::probe_browser_session(window).await {
                if !probe.authenticated {
                    return Err(AppError::Message(
                        "Mnelyra's ChatGPT session is no longer authenticated; open Web Models and sign in again"
                            .into(),
                    ));
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(75)).await;
    }
    Err(AppError::Message(
        "Fresh ChatGPT Temporary Chat did not become ready".into(),
    ))
}

async fn browser_turn_ready(window: &WebviewWindow) -> AppResult<bool> {
    let expression = r#"(() => {
      const visible = (element) => {
        if (!element || !element.isConnected) return false;
        const style = getComputedStyle(element); const rect = element.getBoundingClientRect();
        return rect.width > 0 && rect.height > 0 && style.display !== 'none'
          && style.visibility !== 'hidden' && style.opacity !== '0';
      };
      const composer = Array.from(document.querySelectorAll(
        '[data-testid="prompt-textarea"], #prompt-textarea, [contenteditable="true"][data-lexical-editor="true"], [contenteditable="true"][role="textbox"], textarea'
      )).find(visible);
      const effortPicker = Array.from(document.querySelectorAll(
        'button.__composer-pill[aria-haspopup="menu"], button[data-testid="model-switcher-dropdown-button"][aria-haspopup="menu"]'
      )).find(visible);
      const current = new URL(location.href);
      return Boolean(composer && effortPicker)
        && current.origin === 'https://chatgpt.com'
        && current.pathname === '/'
        && current.searchParams.get('temporary-chat') === 'true';
    })()"#;
    let value = super::evaluate_browser_value(window, expression).await?;
    Ok(value.as_bool().unwrap_or(false))
}

fn effort_index(request: &Value) -> AppResult<usize> {
    let model = request
        .get("model")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::Message("Web-model Responses request is missing model".into()))?;
    match model {
        "mnelyra-web/low" | "mnelyra-web/instant" => return Ok(0),
        "mnelyra-web/medium" => return Ok(1),
        "mnelyra-web/high" => return Ok(2),
        "gpt-5.6-sol" => {}
        _ => {
            return Err(AppError::Message(format!(
                "Unsupported Mnelyra Web model: {model}"
            )))
        }
    }

    let effort = request
        .pointer("/reasoning/effort")
        .and_then(Value::as_str)
        .or_else(|| request.get("reasoning_effort").and_then(Value::as_str))
        .or_else(|| request.get("reasoningEffort").and_then(Value::as_str))
        .unwrap_or("low");
    match effort {
        "low" | "minimal" | "none" => Ok(0),
        "medium" => Ok(1),
        "high" => Ok(2),
        "xhigh" | "max" | "ultra" => Err(AppError::Message(format!(
            "Mnelyra Web Models currently maps ChatGPT's native Low, Medium, and High effort levels; Codex requested {effort}"
        ))),
        _ => Err(AppError::Message(format!(
            "Unsupported Codex reasoning effort for Mnelyra Web Models: {effort}"
        ))),
    }
}

async fn select_effort(window: &WebviewWindow, target: usize) -> AppResult<()> {
    let expression = r#"(async () => {
      const visible = (element) => {
        if (!element || !element.isConnected) return false;
        const style = getComputedStyle(element);
        const rect = element.getBoundingClientRect();
        return rect.width > 0 && rect.height > 0 && style.display !== 'none'
          && style.visibility !== 'hidden' && style.opacity !== '0';
      };
      const normalize = (value) => String(value || '').replace(/\s+/g, ' ').trim().toLowerCase();
      const fieldValues = (element) => [
        element?.innerText,
        element?.textContent,
        element?.getAttribute?.('aria-label'),
        element?.getAttribute?.('title'),
        element?.getAttribute?.('data-testid'),
        element?.getAttribute?.('data-value'),
        element?.getAttribute?.('value'),
      ].map(normalize).filter(Boolean);
      const descriptor = (element) => {
        const values = fieldValues(element);
        const classes = typeof element?.className === 'string' ? element.className : '';
        const html = String(element?.outerHTML || '').replace(/\s+/g, ' ').slice(0, 260);
        return [
          element?.tagName?.toLowerCase() || '?',
          `role=${element?.getAttribute?.('role') || ''}`,
          `testid=${element?.getAttribute?.('data-testid') || ''}`,
          `aria=${element?.getAttribute?.('aria-label') || ''}`,
          `popup=${element?.getAttribute?.('aria-haspopup') || ''}`,
          `state=${element?.getAttribute?.('data-state') || ''}`,
          `class=${classes.slice(0, 140)}`,
          `text=${(values[0] || '').slice(0, 80)}`,
          `html=${html}`,
        ].join('|');
      };
      const target = __MNELYRA_TARGET__;
      const aliasGroups = [
        ['instant', '即时', '即時'],
        ['medium', '中等', '标准', '標準', 'standard'],
        ['high', '高', '扩展', '擴展', 'extended'],
      ];
      const aliases = aliasGroups[target] || [];
      const aliasMatches = (value, alias) => {
        if (value === alias) return true;
        if (value.startsWith(`${alias} `) || value.startsWith(`${alias}\n`)) return true;
        if (value.endsWith(` ${alias}`) || value.endsWith(`\n${alias}`)) return true;
        if (value.endsWith(`/${alias}`) || value.endsWith(`-${alias}`) || value.endsWith(`_${alias}`)) return true;
        if (value.includes(` ${alias} `)) return true;
        // Current localized picker rows include descriptive copy after the
        // short CJK label. Prefix matching keeps 高 distinct from 超高.
        if (/^[\u3400-\u9fff\uf900-\ufaff]/.test(alias) && value.startsWith(alias)) return true;
        return false;
      };
      const fieldMatchesTarget = (element) => fieldValues(element).some((value) =>
        aliases.some((alias) => aliasMatches(value, alias))
      );
      const anyEffortLabel = (element) => fieldValues(element).some((value) =>
        aliasGroups.flat().some((alias) => value === alias || value.includes(alias))
        || /reasoning|thinking|effort|intelligence|推理|思考/.test(value)
      );
      const composer = Array.from(document.querySelectorAll(
        '[data-testid="prompt-textarea"], #prompt-textarea, [contenteditable="true"][data-lexical-editor="true"], [contenteditable="true"][role="textbox"], textarea'
      )).find(visible);
      if (!composer) return { available: false, diagnostics: ['composer-unavailable'] };

      const composerRect = composer.getBoundingClientRect();
      const allControls = Array.from(document.querySelectorAll(
        'button, [role="button"], [role="menuitem"], [role="option"], [role="radio"], [role="menuitemradio"], [role="slider"]'
      )).filter(visible);
      const nearComposer = allControls.filter((element) => {
        const rect = element.getBoundingClientRect();
        const centerY = rect.top + rect.height / 2;
        const horizontalOverlap = rect.right >= composerRect.left - 160 && rect.left <= composerRect.right + 160;
        return horizontalOverlap && centerY >= composerRect.top - 140 && centerY <= composerRect.bottom + 160;
      });
      const diagnostics = () => {
        const namedNearby = Array.from(document.querySelectorAll(
          '[data-testid], [aria-label], [role], [title], [data-state], [data-value]'
        )).filter(visible).filter((element) => {
          const rect = element.getBoundingClientRect();
          const centerY = rect.top + rect.height / 2;
          const horizontalOverlap = rect.right >= composerRect.left - 240 && rect.left <= composerRect.right + 240;
          return horizontalOverlap && centerY >= composerRect.top - 260 && centerY <= composerRect.bottom + 220;
        });
        const ancestry = [];
        let node = composer;
        for (let depth = 0; node && depth < 6; depth += 1, node = node.parentElement) {
          ancestry.push(`ancestor${depth}:${descriptor(node)}`);
        }
        return ancestry.concat(namedNearby.slice(0, 60).map(descriptor));
      };

      // The current web picker renders the selected effort directly in the
      // composer. If it already matches, do not reopen the menu.
      const alreadySelected = nearComposer.find((candidate) =>
        fieldMatchesTarget(candidate)
        && !['menuitem', 'option', 'radio', 'menuitemradio'].includes(candidate.getAttribute('role') || '')
      );
      if (alreadySelected) {
        return { available: true, kind: 'already', diagnostics: diagnostics() };
      }

      const composerControls = nearComposer.filter((element) =>
        ['button', 'div'].includes(element.tagName.toLowerCase())
        || element.getAttribute('role') === 'button'
      );
      const explicitControl = composerControls.find((candidate) => {
        const testid = normalize(candidate.getAttribute('data-testid'));
        return /model|reasoning|thinking|effort|intelligence/.test(testid) || anyEffortLabel(candidate);
      });
      const menuControl = composerControls.find((candidate) => {
        const joined = fieldValues(candidate).join(' ');
        return candidate.getAttribute('aria-haspopup')
          && !/attach|upload|voice|send|account|profile|sidebar|history|search/.test(joined);
      });
      const fallbackControl = composerControls.find((candidate) => {
        if (candidate.tagName.toLowerCase() !== 'button') return false;
        const joined = fieldValues(candidate).join(' ');
        const testid = normalize(candidate.getAttribute('data-testid'));
        if (/composer-plus|send|submit|upload|attach|dictat|microphone|voice/.test(`${joined} ${testid}`)) return false;
        if (/添加文件|上传|听写|语音|发送|附件/.test(joined)) return false;
        return true;
      });
      const control = explicitControl || menuControl || fallbackControl;
      if (!control) return { available: false, diagnostics: diagnostics() };
      const rect = control.getBoundingClientRect();
      return {
        available: true,
        kind: 'trigger',
        x: rect.left + rect.width / 2,
        y: rect.top + rect.height / 2,
        diagnostics: diagnostics(),
      };
    })()"#
        .replace("__MNELYRA_TARGET__", &target.to_string());
    let mut surface: EffortSurface =
        serde_json::from_value(super::evaluate_browser_value(window, &expression).await?)?;
    if !surface.available {
        if target == 0 {
            return Ok(());
        }
        let diagnostics = surface.diagnostics.as_deref().unwrap_or(&[]).join(" || ");
        return Err(AppError::Message(format!(
            "Mnelyra could not locate ChatGPT's current Medium/High picker controls{}",
            if diagnostics.is_empty() {
                String::new()
            } else {
                format!("; visible controls: {diagnostics}")
            }
        )));
    }

    if surface.kind.as_deref() == Some("trigger") {
        let x = surface.x.ok_or_else(|| {
            AppError::Message("ChatGPT effort picker did not expose a click X coordinate".into())
        })?;
        let y = surface.y.ok_or_else(|| {
            AppError::Message("ChatGPT effort picker did not expose a click Y coordinate".into())
        })?;
        dispatch_mouse_click(window, x, y).await?;
        tokio::time::sleep(Duration::from_millis(120)).await;
        surface = read_open_effort_surface(window, target).await?;
        if !surface.available && focus_effort_picker(window).await.is_ok() {
            dispatch_key(window, " ", "Space", 32, 0).await?;
            tokio::time::sleep(Duration::from_millis(120)).await;
            surface = read_open_effort_surface(window, target).await?;
        }
        if !surface.available
            && dispatch_effort_picker_pointer_sequence(window)
                .await
                .is_ok()
        {
            tokio::time::sleep(Duration::from_millis(120)).await;
            surface = read_open_effort_surface(window, target).await?;
        }
        if !surface.available {
            let diagnostics = surface.diagnostics.as_deref().unwrap_or(&[]).join(" || ");
            return Err(AppError::Message(format!(
                "Mnelyra opened ChatGPT's effort picker but could not locate the requested option{}",
                if diagnostics.is_empty() {
                    String::new()
                } else {
                    format!("; visible controls: {diagnostics}")
                }
            )));
        }
    }

    match surface.kind.as_deref() {
        Some("already") => {}
        Some("semantic-click") => {
            let x = surface.x.ok_or_else(|| {
                AppError::Message(
                    "ChatGPT effort option did not expose a click X coordinate".into(),
                )
            })?;
            let y = surface.y.ok_or_else(|| {
                AppError::Message(
                    "ChatGPT effort option did not expose a click Y coordinate".into(),
                )
            })?;
            dispatch_mouse_click(window, x, y).await?;
        }
        Some("slider") => {
            let min = surface.min.ok_or_else(|| {
                AppError::Message("ChatGPT reasoning slider did not expose aria-valuemin".into())
            })?;
            let max = surface.max.ok_or_else(|| {
                AppError::Message("ChatGPT reasoning slider did not expose aria-valuemax".into())
            })?;
            let _current = surface.value.ok_or_else(|| {
                AppError::Message("ChatGPT reasoning slider did not expose aria-valuenow".into())
            })?;
            let desired = min + target as i64;
            if desired > max {
                return Err(AppError::Message(format!(
                    "ChatGPT reasoning slider range {min}..={max} cannot represent requested index {target}"
                )));
            }
            let focus = r#"(() => {
              const visible = (element) => {
                if (!element || !element.isConnected) return false;
                const style = getComputedStyle(element); const rect = element.getBoundingClientRect();
                return rect.width > 0 && rect.height > 0 && style.display !== 'none'
                  && style.visibility !== 'hidden' && style.opacity !== '0';
              };
              const slider = Array.from(document.querySelectorAll('[data-model-reasoning-effort-slider] [role="slider"]')).find(visible);
              if (!slider) throw new Error('ChatGPT reasoning slider disappeared');
              slider.focus(); return true;
            })()"#;
            let _ = super::evaluate_browser_value(window, focus).await?;
            dispatch_key(window, "Home", "Home", 36, 0).await?;
            for _ in min..desired {
                dispatch_key(window, "ArrowRight", "ArrowRight", 39, 0).await?;
            }
        }
        other => {
            return Err(AppError::Message(format!(
                "Unsupported ChatGPT reasoning control: {other:?}"
            )));
        }
    }
    tokio::time::sleep(UI_SETTLE).await;
    Ok(())
}

async fn focus_effort_picker(window: &WebviewWindow) -> AppResult<()> {
    let expression = r#"(() => {
      const visible = (element) => {
        if (!element || !element.isConnected) return false;
        const style = getComputedStyle(element); const rect = element.getBoundingClientRect();
        return rect.width > 0 && rect.height > 0 && style.display !== 'none'
          && style.visibility !== 'hidden' && style.opacity !== '0';
      };
      const trigger = Array.from(document.querySelectorAll('button.__composer-pill[aria-haspopup="menu"]')).find(visible);
      if (!trigger) throw new Error('ChatGPT effort picker trigger disappeared');
      trigger.focus();
      return document.activeElement === trigger;
    })()"#;
    let value = super::evaluate_browser_value(window, expression).await?;
    if value.as_bool().unwrap_or(false) {
        Ok(())
    } else {
        Err(AppError::Message(
            "ChatGPT effort picker could not receive keyboard focus".into(),
        ))
    }
}

async fn dispatch_effort_picker_pointer_sequence(window: &WebviewWindow) -> AppResult<()> {
    let expression = r#"(() => {
      const visible = (element) => {
        if (!element || !element.isConnected) return false;
        const style = getComputedStyle(element); const rect = element.getBoundingClientRect();
        return rect.width > 0 && rect.height > 0 && style.display !== 'none'
          && style.visibility !== 'hidden' && style.opacity !== '0';
      };
      const trigger = Array.from(document.querySelectorAll('button.__composer-pill[aria-haspopup="menu"]')).find(visible);
      if (!trigger) throw new Error('ChatGPT effort picker trigger disappeared');
      const rect = trigger.getBoundingClientRect();
      const init = {
        bubbles: true, cancelable: true, composed: true,
        clientX: rect.left + rect.width / 2,
        clientY: rect.top + rect.height / 2,
        button: 0,
      };
      trigger.dispatchEvent(new PointerEvent('pointerdown', { ...init, pointerId: 1, pointerType: 'mouse', isPrimary: true, buttons: 1 }));
      trigger.dispatchEvent(new MouseEvent('mousedown', { ...init, buttons: 1 }));
      trigger.dispatchEvent(new PointerEvent('pointerup', { ...init, pointerId: 1, pointerType: 'mouse', isPrimary: true, buttons: 0 }));
      trigger.dispatchEvent(new MouseEvent('mouseup', { ...init, buttons: 0 }));
      trigger.dispatchEvent(new MouseEvent('click', { ...init, buttons: 0 }));
      return true;
    })()"#;
    let _ = super::evaluate_browser_value(window, expression).await?;
    Ok(())
}

async fn read_open_effort_surface(
    window: &WebviewWindow,
    target: usize,
) -> AppResult<EffortSurface> {
    let expression = r#"(async () => {
      const visible = (element) => {
        if (!element || !element.isConnected) return false;
        const style = getComputedStyle(element);
        const rect = element.getBoundingClientRect();
        return rect.width > 0 && rect.height > 0 && style.display !== 'none'
          && style.visibility !== 'hidden' && style.opacity !== '0';
      };
      const normalize = (value) => String(value || '').replace(/\s+/g, ' ').trim().toLowerCase();
      const fieldValues = (element) => [
        element?.innerText,
        element?.textContent,
        element?.getAttribute?.('aria-label'),
        element?.getAttribute?.('title'),
        element?.getAttribute?.('data-testid'),
        element?.getAttribute?.('data-value'),
        element?.getAttribute?.('value'),
      ].map(normalize).filter(Boolean);
      const target = __MNELYRA_TARGET__;
      const aliases = [
        ['instant', '即时', '即時'],
        ['medium', '中等', '标准', '標準', 'standard'],
        ['high', '高', '扩展', '擴展', 'extended'],
      ][target] || [];
      const aliasMatches = (value, alias) => {
        if (value === alias) return true;
        if (value.startsWith(`${alias} `) || value.startsWith(`${alias}\n`)) return true;
        if (value.endsWith(`/${alias}`) || value.endsWith(`-${alias}`) || value.endsWith(`_${alias}`)) return true;
        if (value.includes(` ${alias} `)) return true;
        if (/^[\u3400-\u9fff\uf900-\ufaff]/.test(alias) && value.startsWith(alias)) return true;
        return false;
      };
      const matchesTarget = (element) => fieldValues(element).some((value) =>
        aliases.some((alias) => aliasMatches(value, alias))
      );
      const aliasGroups = [
        ['instant', '即时', '即時', '快速', 'fast'],
        ['medium', '中等', '标准', '標準', 'standard'],
        ['high', '高', '扩展', '擴展', 'extended', '深度', '深入'],
      ];
      const describe = (element) => [
        element?.tagName?.toLowerCase() || '?',
        `role=${element?.getAttribute?.('role') || ''}`,
        `testid=${element?.getAttribute?.('data-testid') || ''}`,
        `state=${element?.getAttribute?.('data-state') || ''}`,
        `text=${normalize(element?.innerText || element?.textContent).slice(0, 120)}`,
      ].join('|');
      const triggerDiagnostic = () => {
        const trigger = Array.from(document.querySelectorAll('button.__composer-pill[aria-haspopup="menu"]')).find(visible);
        return trigger ? `trigger:${describe(trigger)}` : 'trigger:missing';
      };
      const menuRoots = () => Array.from(document.querySelectorAll(
        '[role="menu"], [role="listbox"], [data-radix-menu-content], [data-state="open"]'
      )).filter(visible).filter((root) => {
        const rows = root.querySelectorAll('[role="menuitemradio"], [role="menuitem"], [role="option"], [role="radio"]');
        if (rows.length >= 3) return true;
        const text = normalize(root.innerText || root.textContent);
        const matchedGroups = aliasGroups.filter((group) => group.some((alias) => text.includes(alias))).length;
        return matchedGroups >= 2;
      });
      const scopedRows = (root) => Array.from(root.querySelectorAll(
        '[role="menuitemradio"], [role="menuitem"], [role="option"], [role="radio"]'
      )).filter(visible);
      // The hidden WebView can take longer to materialize the Radix menu than
      // a foreground browser. Keep the proven tolerance here: reliability is
      // more important than shaving a fraction of a second off selector setup.
      const deadline = Date.now() + 1500;
      while (Date.now() < deadline) {
        const roots = menuRoots();
        for (const root of roots) {
          const rows = scopedRows(root);
          const targetOption = rows.find(matchesTarget)
            || Array.from(root.querySelectorAll('button, [role="button"]')).filter(visible).find(matchesTarget);
          if (targetOption) {
            const rect = targetOption.getBoundingClientRect();
            return {
              available: true,
              kind: 'semantic-click',
              x: rect.left + rect.width / 2,
              y: rect.top + rect.height / 2,
              diagnostics: [triggerDiagnostic(), `menu:${describe(root)}`].concat(rows.slice(0, 12).map(describe)),
            };
          }

          // Current ChatGPT exposes exactly three reasoning choices in order.
          // If localization changes the row text but the open picker still
          // presents a three-choice radio/menu group, use that structural
          // contract instead of failing on copy changes.
          if (rows.length === 3 && target < rows.length) {
            const row = rows[target];
            const rect = row.getBoundingClientRect();
            return {
              available: true,
              kind: 'semantic-click',
              x: rect.left + rect.width / 2,
              y: rect.top + rect.height / 2,
              diagnostics: [triggerDiagnostic(), `menu-index-fallback:${describe(root)}`].concat(rows.map(describe)),
            };
          }
        }
        const slider = Array.from(document.querySelectorAll(
          '[data-model-reasoning-effort-slider] [role="slider"], [role="slider"][aria-valuemin][aria-valuemax]'
        )).find(visible);
        if (slider) return {
          available: true, kind: 'slider',
          min: Number(slider.getAttribute('aria-valuemin')),
          max: Number(slider.getAttribute('aria-valuemax')),
          value: Number(slider.getAttribute('aria-valuenow')),
          diagnostics: [triggerDiagnostic(), `slider:${describe(slider)}`],
        };
        await new Promise(resolve => setTimeout(resolve, 75));
      }
      const visibleOptions = Array.from(document.querySelectorAll(
        'button, [role="button"], [role="menuitem"], [role="option"], [role="radio"], [role="menuitemradio"]'
      )).filter(visible);
      const roots = menuRoots();
      return {
        available: false,
        diagnostics: [triggerDiagnostic()]
          .concat(roots.slice(0, 6).map(root => `menu:${describe(root)}`))
          .concat(visibleOptions.slice(-30).map(describe)),
      };
    })()"#
        .replace("__MNELYRA_TARGET__", &target.to_string());
    serde_json::from_value(super::evaluate_browser_value(window, &expression).await?)
        .map_err(AppError::from)
}

async fn dispatch_mouse_click(window: &WebviewWindow, x: f64, y: f64) -> AppResult<()> {
    for event_type in ["mousePressed", "mouseReleased"] {
        let _ = super::call_devtools(
            window,
            "Input.dispatchMouseEvent",
            json!({
                "type": event_type,
                "x": x,
                "y": y,
                "button": "left",
                "buttons": if event_type == "mousePressed" { 1 } else { 0 },
                "clickCount": 1,
            }),
        )
        .await?;
    }
    Ok(())
}

async fn attach_prompt(window: &WebviewWindow, prompt: &str) -> AppResult<ComposerState> {
    let expected = normalize_editor_text(prompt);
    for attempt in 0..2 {
        focus_and_clear_composer(window).await?;
        for chunk in prompt_chunks(prompt, INSERT_CHUNK_CHARS) {
            let _ =
                super::call_devtools(window, "Input.insertText", json!({"text": chunk})).await?;
        }
        tokio::time::sleep(UI_SETTLE).await;
        let state = composer_state(window).await?;
        let observed = normalize_editor_text(&state.text);
        if observed == expected && state.send_ready {
            return Ok(state);
        }
        if attempt == 1 {
            let first_diff = first_difference_char(&expected, &observed)
                .map(|index| index.to_string())
                .unwrap_or_else(|| "none".into());
            return Err(AppError::Message(format!(
                "ChatGPT composer did not preserve the Codex prompt exactly (canonical expected {} chars, canonical observed {} chars, browser innerText {} chars, first difference at char {})",
                expected.chars().count(),
                observed.chars().count(),
                state.inner_text_len,
                first_diff
            )));
        }
    }
    unreachable!()
}

async fn focus_and_clear_composer(window: &WebviewWindow) -> AppResult<()> {
    let expression = r#"(() => {
      const visible = (element) => {
        if (!element || !element.isConnected) return false;
        const style = getComputedStyle(element); const rect = element.getBoundingClientRect();
        return rect.width > 0 && rect.height > 0 && style.display !== 'none'
          && style.visibility !== 'hidden' && style.opacity !== '0';
      };
      const composer = Array.from(document.querySelectorAll(
        '[data-testid="prompt-textarea"], #prompt-textarea, [contenteditable="true"][data-lexical-editor="true"], [contenteditable="true"][role="textbox"], textarea'
      )).find(visible);
      if (!composer) throw new Error('ChatGPT composer is unavailable');
      composer.focus(); return true;
    })()"#;
    let _ = super::evaluate_browser_value(window, expression).await?;
    dispatch_key(window, "a", "KeyA", 65, 2).await?;
    dispatch_key(window, "Backspace", "Backspace", 8, 0).await?;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let state = composer_state(window).await?;
    if !normalize_editor_text(&state.text).is_empty() {
        return Err(AppError::Message(
            "ChatGPT composer could not be cleared before attaching Codex context".into(),
        ));
    }
    Ok(())
}

async fn composer_state(window: &WebviewWindow) -> AppResult<ComposerState> {
    let expression = r#"(() => {
      const visible = (element) => {
        if (!element || !element.isConnected) return false;
        const style = getComputedStyle(element); const rect = element.getBoundingClientRect();
        return rect.width > 0 && rect.height > 0 && style.display !== 'none'
          && style.visibility !== 'hidden' && style.opacity !== '0';
      };
      const composer = Array.from(document.querySelectorAll(
        '[data-testid="prompt-textarea"], #prompt-textarea, [contenteditable="true"][data-lexical-editor="true"], [contenteditable="true"][role="textbox"], textarea'
      )).find(visible);
      if (!composer) throw new Error('ChatGPT composer is unavailable');
      const codePointLength = (value) => Array.from(String(value || '')).length;
      const inlineText = (node) => {
        if (!node) return '';
        if (node.nodeType === Node.TEXT_NODE) return node.nodeValue || '';
        if (node.nodeType !== Node.ELEMENT_NODE) return '';
        if (node.tagName === 'BR') return '\n';
        return Array.from(node.childNodes).map(inlineText).join('');
      };
      const canonicalEditorText = (root) => {
        if (root.tagName === 'TEXTAREA') return root.value;
        const children = Array.from(root.childNodes);
        const blockTag = (node) => node?.nodeType === Node.ELEMENT_NODE
          && /^(P|DIV|LI)$/.test(node.tagName || '');
        if (!children.some(blockTag)) return children.map(inlineText).join('');
        return children.map((node) => {
          if (!blockTag(node)) return inlineText(node);
          let value = inlineText(node);
          // Lexical uses a lone trailing <br> as an empty-paragraph caret
          // placeholder. Paragraph boundaries are represented by the join
          // below, so the placeholder must not create a second newline.
          if (node.childNodes.length === 1 && node.firstChild?.nodeName === 'BR') return '';
          if (value.endsWith('\n')) value = value.slice(0, -1);
          return value;
        }).join('\n');
      };
      const form = composer.closest('form');
      const send = Array.from((form || document).querySelectorAll(
        'button[data-testid="send-button"], button[aria-label*="Send" i], button[type="submit"]'
      )).find(element => visible(element) && element.getAttribute('data-testid') !== 'stop-button');
      const innerText = composer.tagName === 'TEXTAREA' ? composer.value : composer.innerText;
      return {
        text: canonicalEditorText(composer),
        innerTextLen: codePointLength(innerText),
        userTurnCount: document.querySelectorAll(
          '[data-testid^="conversation-turn-"][data-turn="user"], [data-testid^="conversation-turn-"][data-message-author-role="user"], [data-testid^="conversation-turn-"]:has([data-message-author-role="user"])'
        ).length,
        assistantTurnCount: document.querySelectorAll(
          '[data-testid^="conversation-turn-"][data-turn="assistant"], [data-testid^="conversation-turn-"][data-message-author-role="assistant"], [data-testid^="conversation-turn-"]:has([data-message-author-role="assistant"])'
        ).length,
        sendReady: Boolean(send && !send.disabled && send.getAttribute('aria-disabled') !== 'true'),
      };
    })()"#;
    let value = super::evaluate_browser_value(window, expression).await?;
    serde_json::from_value(value).map_err(AppError::from)
}

async fn submit_prompt(window: &WebviewWindow, initial: &ComposerState) -> AppResult<()> {
    if !initial.send_ready {
        return Err(AppError::Message(
            "ChatGPT send button is not ready after attaching the Codex prompt".into(),
        ));
    }
    let expression = r#"(() => {
      const visible = (element) => {
        if (!element || !element.isConnected) return false;
        const style = getComputedStyle(element); const rect = element.getBoundingClientRect();
        return rect.width > 0 && rect.height > 0 && style.display !== 'none'
          && style.visibility !== 'hidden' && style.opacity !== '0';
      };
      const composer = Array.from(document.querySelectorAll(
        '[data-testid="prompt-textarea"], #prompt-textarea, [contenteditable="true"][data-lexical-editor="true"], [contenteditable="true"][role="textbox"], textarea'
      )).find(visible);
      const form = composer?.closest('form');
      const send = Array.from((form || document).querySelectorAll(
        'button[data-testid="send-button"], button[aria-label*="Send" i], button[type="submit"]'
      )).find(element => visible(element) && element.getAttribute('data-testid') !== 'stop-button');
      if (!send || send.disabled || send.getAttribute('aria-disabled') === 'true') {
        throw new Error('ChatGPT send button is unavailable');
      }
      send.click(); return true;
    })()"#;
    let _ = super::evaluate_browser_value(window, expression).await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while tokio::time::Instant::now() < deadline {
        let probe = read_turn_probe(window).await?;
        if probe.user_turn_count > initial.user_turn_count
            || probe.assistant_turn_count > initial.assistant_turn_count
        {
            return Ok(());
        }
        if probe.running || probe.assistant_present {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(75)).await;
    }
    Err(AppError::Message(
        "ChatGPT did not acknowledge the submitted Codex prompt".into(),
    ))
}

async fn stream_completed_markdown(
    window: &WebviewWindow,
    delta_tx: &mpsc::Sender<String>,
    turn_started: Instant,
) -> AppResult<String> {
    let deadline = tokio::time::Instant::now() + TURN_TIMEOUT;
    let mut emitted = String::new();
    let mut committed_blocks: Vec<String> = Vec::new();
    let mut last_blocks: Vec<String> = Vec::new();
    let mut last_markdown = String::new();
    let mut last_render_signature = String::new();
    let mut last_probe_signature = String::new();
    let mut last_render_at: Option<Instant> = None;
    let mut stable_polls = 0_u8;
    let mut saw_assistant = false;
    let mut first_assistant_logged = false;
    let mut first_delta_logged = false;
    while tokio::time::Instant::now() < deadline {
        let probe = read_turn_probe(window).await?;
        if let Some(error) = probe.error.filter(|value| !value.trim().is_empty()) {
            return Err(AppError::Message(error));
        }
        if probe.assistant_present {
            saw_assistant = true;
            if !first_assistant_logged {
                first_assistant_logged = true;
                eprintln!(
                    "[web-models timing] phase=first_assistant_ms value={}",
                    turn_started.elapsed().as_millis()
                );
            }
        }

        if saw_assistant
            && !probe.content_signature.is_empty()
            && probe.content_signature == last_probe_signature
        {
            stable_polls = stable_polls.saturating_add(1);
        } else {
            stable_polls = 0;
        }
        last_probe_signature = probe.content_signature.clone();

        let completed = saw_assistant
            && !probe.running
            && probe.content_text_len > 0
            && (probe.completion_action_visible || stable_polls >= COMPLETION_STABLE_POLLS);

        let render_due = if !saw_assistant {
            false
        } else if completed || last_render_at.is_none() {
            true
        } else {
            let interval_elapsed =
                last_render_at.is_some_and(|at| at.elapsed() >= STREAM_RENDER_INTERVAL);
            interval_elapsed && probe.content_signature != last_render_signature
        };

        if render_due {
            let state = read_turn_state(window).await?;
            let markdown = state.markdown;

            if completed {
                if markdown.starts_with(&emitted) {
                    return Ok(markdown);
                }

                if let Some(reconciled) = reconcile_streamed_with_final(&emitted, &markdown) {
                    eprintln!(
                        "[web-models] final DOM rewrote streamed Markdown; preserved the emitted prefix and reconciled from a stable suffix anchor"
                    );
                    return Ok(reconciled);
                }

                let committed_count = committed_blocks.len();
                if state.blocks.len() >= committed_count {
                    let mut reconciled_blocks = committed_blocks.clone();
                    reconciled_blocks.extend_from_slice(&state.blocks[committed_count..]);
                    let reconciled = normalize_rendered_blocks(&reconciled_blocks);
                    if reconciled.starts_with(&emitted) {
                        eprintln!(
                            "[web-models] final DOM rewrote committed Markdown; preserved {} streamed block(s) and reconciled the final tail",
                            committed_count
                        );
                        return Ok(reconciled);
                    }
                }

                eprintln!(
                    "[web-models] final DOM shrank across committed blocks; preserving streamed output instead of disconnecting"
                );
                return Ok(emitted);
            }

            if !state.blocks.is_empty() {
                let safe_count = state.streamable_count.min(state.blocks.len());
                let committed_count = committed_blocks.len();
                let committed_still_matches = state.blocks.len() >= committed_count
                    && state.blocks[..committed_count] == committed_blocks[..];
                let stable_safe_blocks = safe_count > committed_count
                    && last_blocks.len() >= safe_count
                    && state.blocks[..safe_count] == last_blocks[..safe_count];

                if committed_still_matches && stable_safe_blocks {
                    let candidate_blocks = state.blocks[..safe_count].to_vec();
                    let candidate = normalize_rendered_blocks(&candidate_blocks);
                    if candidate.starts_with(&emitted) && candidate.len() > emitted.len() {
                        let delta = candidate[emitted.len()..].to_string();
                        delta_tx.send(delta.clone()).await.map_err(|_| {
                            AppError::Message(
                                "Codex closed the Mnelyra Web Models response stream".into(),
                            )
                        })?;
                        if !first_delta_logged {
                            first_delta_logged = true;
                            eprintln!(
                                "[web-models timing] phase=first_codex_delta_ms value={}",
                                turn_started.elapsed().as_millis()
                            );
                        }
                        emitted.push_str(&delta);
                        committed_blocks = candidate_blocks;
                    }
                }

                // A single long prose block used to be held until completion.
                // Stream the prefix that survived two full Markdown renders,
                // while keeping a generous active tail for ChatGPT DOM rewrites.
                if !last_markdown.is_empty()
                    && markdown.starts_with(&emitted)
                    && last_markdown.starts_with(&emitted)
                {
                    let common_bytes = common_prefix_byte_len(&last_markdown, &markdown);
                    let candidate_end =
                        stable_stream_end(&markdown, common_bytes, STREAM_TEXT_TAIL_HOLD_CHARS);
                    if candidate_end > emitted.len()
                        && markdown.is_char_boundary(emitted.len())
                        && markdown.is_char_boundary(candidate_end)
                    {
                        let delta = markdown[emitted.len()..candidate_end].to_string();
                        delta_tx.send(delta.clone()).await.map_err(|_| {
                            AppError::Message(
                                "Codex closed the Mnelyra Web Models response stream".into(),
                            )
                        })?;
                        if !first_delta_logged {
                            first_delta_logged = true;
                            eprintln!(
                                "[web-models timing] phase=first_codex_delta_ms value={}",
                                turn_started.elapsed().as_millis()
                            );
                        }
                        emitted.push_str(&delta);
                    }
                }
                last_blocks = state.blocks;
            }
            last_markdown = markdown;
            last_render_signature = probe.content_signature;
            last_render_at = Some(Instant::now());
        }
        tokio::time::sleep(STREAM_POLL_INTERVAL).await;
    }
    Err(AppError::Message(
        "ChatGPT Web-model turn exceeded the Mnelyra browser timeout".into(),
    ))
}

async fn read_turn_probe(window: &WebviewWindow) -> AppResult<TurnProbe> {
    let expression = r#"(() => {
      const visible = (element) => {
        if (!element || !element.isConnected) return false;
        const style = getComputedStyle(element); const rect = element.getBoundingClientRect();
        return rect.width > 0 && rect.height > 0 && style.display !== 'none'
          && style.visibility !== 'hidden' && style.opacity !== '0';
      };
      const assistantSelector = [
        '[data-testid^="conversation-turn-"][data-turn="assistant"]',
        '[data-testid^="conversation-turn-"][data-message-author-role="assistant"]',
        '[data-testid^="conversation-turn-"]:has([data-message-author-role="assistant"])'
      ].join(', ');
      const userSelector = [
        '[data-testid^="conversation-turn-"][data-turn="user"]',
        '[data-testid^="conversation-turn-"][data-message-author-role="user"]',
        '[data-testid^="conversation-turn-"]:has([data-message-author-role="user"])'
      ].join(', ');
      const turns = Array.from(document.querySelectorAll(assistantSelector));
      const turn = turns.at(-1) || null;
      const content = turn?.querySelector('.markdown, [class*="markdown"], [data-message-author-role="assistant"]') || turn;
      const text = content?.textContent || '';
      let tailHash = 2166136261;
      for (let index = Math.max(0, text.length - 64); index < text.length; index += 1) {
        tailHash ^= text.charCodeAt(index);
        tailHash = Math.imul(tailHash, 16777619) >>> 0;
      }
      const formatNodeCount = content
        ? content.querySelectorAll('strong, b, em, i, code, pre, a, li, table, blockquote, h1, h2, h3, h4, h5, h6').length
        : 0;
      const running = Array.from(document.querySelectorAll('[data-testid="stop-button"]')).some(visible);
      const completionActionVisible = turn
        ? Array.from(turn.querySelectorAll('button[data-testid="copy-turn-action-button"]')).some(visible)
        : false;
      const alerts = Array.from(document.querySelectorAll('[role="alert"], [role="dialog"]')).filter(visible)
        .map(element => (element.innerText || '').trim()).filter(Boolean);
      const error = alerts.find(text => /Something went wrong|Too many requests|Failed to load subscription/i.test(text)) || null;
      return {
        assistantPresent: Boolean(turn),
        running,
        completionActionVisible,
        contentSignature: content ? `${text.length}:${tailHash}:${formatNodeCount}:${content.childElementCount}` : '',
        contentTextLen: text.length,
        error,
        userTurnCount: document.querySelectorAll(userSelector).length,
        assistantTurnCount: turns.length,
      };
    })()"#;
    let value = super::evaluate_browser_value(window, expression).await?;
    serde_json::from_value(value).map_err(AppError::from)
}

async fn read_turn_state(window: &WebviewWindow) -> AppResult<TurnState> {
    let expression = r#"(() => {
      const escapeInline = (text) => String(text ?? '').replace(/\u00a0/g, ' ');
      const maxTickRun = (text) => Math.max(0, ...Array.from(String(text).matchAll(/`+/g), match => match[0].length));
      const inlineCode = (text) => {
        const raw = escapeInline(text);
        const fence = '`'.repeat(Math.max(1, maxTickRun(raw) + 1));
        return fence + raw + fence;
      };
      const children = (node) => Array.from(node.childNodes).map(child => render(child)).join('');
      const renderTable = (table) => {
        const rows = Array.from(table.querySelectorAll('tr')).map(row =>
          Array.from(row.querySelectorAll(':scope > th, :scope > td')).map(cell => children(cell).trim().replace(/\|/g, '\\|'))
        ).filter(row => row.length > 0);
        if (!rows.length) return '';
        const width = Math.max(...rows.map(row => row.length));
        for (const row of rows) while (row.length < width) row.push('');
        const header = rows[0];
        return '\n\n| ' + header.join(' | ') + ' |\n| ' + header.map(() => '---').join(' | ') + ' |\n'
          + rows.slice(1).map(row => '| ' + row.join(' | ') + ' |').join('\n') + '\n\n';
      };
      const render = (node) => {
        if (node.nodeType === Node.TEXT_NODE) return escapeInline(node.nodeValue);
        if (node.nodeType !== Node.ELEMENT_NODE) return '';
        const el = node;
        if (el.matches('button, svg, style, script, [aria-hidden="true"], [data-testid$="-action-button"]')) return '';
        const tag = el.tagName.toLowerCase();
        if (tag === 'br') return '\n';
        if (tag === 'pre') {
          const codeEl = el.querySelector('code');
          const code = escapeInline((codeEl || el).innerText).replace(/\n$/, '');
          const classes = String(codeEl?.className || el.className || '');
          const language = classes.match(/(?:language-|lang-)([A-Za-z0-9_+.-]+)/)?.[1] || '';
          const fence = '`'.repeat(Math.max(3, maxTickRun(code) + 1));
          return `\n\n${fence}${language}\n${code}\n${fence}\n\n`;
        }
        if (tag === 'code') return inlineCode(el.textContent || '');
        if (tag === 'strong' || tag === 'b') return '**' + children(el) + '**';
        if (tag === 'em' || tag === 'i') return '*' + children(el) + '*';
        if (tag === 'a') {
          const text = children(el).trim() || escapeInline(el.textContent || '');
          const href = el.getAttribute('href');
          return href ? `[${text}](${href})` : text;
        }
        if (/^h[1-6]$/.test(tag)) return '\n\n' + '#'.repeat(Number(tag[1])) + ' ' + children(el).trim() + '\n\n';
        if (tag === 'blockquote') return '\n\n' + children(el).trim().split('\n').map(line => '> ' + line).join('\n') + '\n\n';
        if (tag === 'hr') return '\n\n---\n\n';
        if (tag === 'table') return renderTable(el);
        if (tag === 'ul' || tag === 'ol') {
          const ordered = tag === 'ol';
          const items = Array.from(el.children).filter(child => child.tagName?.toLowerCase() === 'li');
          return '\n' + items.map((item, index) => {
            const marker = ordered ? `${index + 1}. ` : '- ';
            const text = children(item).trim().replace(/\n/g, '\n  ');
            return marker + text;
          }).join('\n') + '\n';
        }
        if (tag === 'li') return children(el);
        const content = children(el);
        if (['p', 'section', 'article'].includes(tag)) return '\n\n' + content.trim() + '\n\n';
        if (tag === 'div' && (el.classList.contains('markdown') || el.getAttribute('data-message-author-role') === 'assistant')) {
          return content;
        }
        return content;
      };
      const assistantSelector = [
        '[data-testid^="conversation-turn-"][data-turn="assistant"]',
        '[data-testid^="conversation-turn-"][data-message-author-role="assistant"]',
        '[data-testid^="conversation-turn-"]:has([data-message-author-role="assistant"])'
      ].join(', ');
      const turns = Array.from(document.querySelectorAll(assistantSelector));
      const turn = turns.at(-1) || null;
      const markdownRoots = turn
        ? Array.from(turn.querySelectorAll('.markdown'))
            .filter(root => !root.parentElement?.closest('.markdown'))
            .filter(root => root.closest('[data-streaming-response-status]') === null)
        : [];
      const fallback = turn?.querySelector('[data-message-author-role="assistant"]') || turn;
      const roots = markdownRoots.length > 0 ? markdownRoots : (fallback ? [fallback] : []);
      const segments = [];
      roots.forEach((root, rootIndex) => {
        const rootIsComplete = rootIndex < roots.length - 1;
        const hasDirectText = Array.from(root.childNodes).some(node =>
          node.nodeType === Node.TEXT_NODE && Boolean(node.textContent?.trim())
        );
        const directChildren = Array.from(root.children);
        if (hasDirectText || directChildren.length === 0) {
          const value = render(root);
          if (value.trim()) segments.push({ markdown: value, streamable: rootIsComplete });
          return;
        }
        directChildren.forEach((child, childIndex) => {
          const tag = child.tagName.toLowerCase();
          const childIsComplete = rootIsComplete || childIndex < directChildren.length - 1;
          const listItems = (tag === 'ul' || tag === 'ol')
            ? Array.from(child.children).filter(item => item.tagName?.toLowerCase() === 'li')
            : [];
          if (listItems.length === 0) {
            const value = render(child);
            if (value.trim()) segments.push({ markdown: value, streamable: childIsComplete });
            return;
          }
          const ordered = tag === 'ol';
          const start = ordered ? Number(child.getAttribute('start') || '1') : 1;
          listItems.forEach((item, itemIndex) => {
            const marker = ordered ? `${start + itemIndex}. ` : '- ';
            const itemText = children(item).trim().replace(/\n/g, '\n  ');
            if (!itemText) return;
            segments.push({
              markdown: `${itemIndex === 0 ? '\n' : ''}${marker}${itemText}\n`,
              streamable: childIsComplete || itemIndex < listItems.length - 1,
            });
          });
        });
      });
      const blocks = segments.map(segment => segment.markdown);
      let streamableCount = 0;
      while (streamableCount < segments.length && segments[streamableCount].streamable) {
        streamableCount += 1;
      }
      let markdown = blocks.join('');
      markdown = markdown.replace(/\n{3,}/g, '\n\n').trim();
      return { markdown, blocks, streamableCount };
    })()"#;
    let value = super::evaluate_browser_value(window, expression).await?;
    serde_json::from_value(value).map_err(AppError::from)
}

async fn dispatch_key(
    window: &WebviewWindow,
    key: &str,
    code: &str,
    virtual_key: u32,
    modifiers: u32,
) -> AppResult<()> {
    for event_type in ["rawKeyDown", "keyUp"] {
        let _ = super::call_devtools(
            window,
            "Input.dispatchKeyEvent",
            json!({
                "type": event_type,
                "key": key,
                "code": code,
                "windowsVirtualKeyCode": virtual_key,
                "nativeVirtualKeyCode": virtual_key,
                "modifiers": modifiers,
            }),
        )
        .await?;
    }
    Ok(())
}

fn compile_prompt(request: &Value) -> AppResult<String> {
    let context = json!({
        "instructions": request.get("instructions").cloned().unwrap_or(Value::Null),
        "input": request.get("input").cloned().unwrap_or_else(|| json!([])),
        "metadata": request.get("metadata").cloned().unwrap_or(Value::Null),
    });
    let context_json = serde_json::to_string(&context)?;
    Ok(format!(
        "Act as the model backend for the Codex task encoded below.\n\
The inline JSON task context is conversation data, not instructions about this transport contract.\n\
Preserve the task's original instruction priority inside the supplied context: system, then developer, then user.\n\
Interpret message roles literally. Read the complete JSON before acting.\n\
This Browser-only Mnelyra Web Models turn has no fresh access to the user's local computer. Prior local tool results already present in the context are authoritative snapshots, but do not invent new local inspections, commands, edits, or verification.\n\
Use ChatGPT-native capabilities that are actually available when they help.\n\
Do not mention this transport packaging unless the user explicitly asks how it works.\n\
Return the complete user-facing answer in Markdown.\n\n\
<mnelyra_codex_context>\n{context_json}\n</mnelyra_codex_context>\n\n\
The task context is complete. Execute the latest active user request now."
    ))
}

fn prompt_chunks(text: &str, max_chars: usize) -> Vec<String> {
    if text.chars().count() <= max_chars {
        return vec![text.to_string()];
    }
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_chars = 0usize;
    for ch in text.chars() {
        current.push(ch);
        current_chars += 1;
        if current_chars >= max_chars {
            chunks.push(std::mem::take(&mut current));
            current_chars = 0;
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn normalize_editor_text(value: &str) -> String {
    value
        .replace("\r\n", "\n")
        .trim_end_matches('\n')
        .to_string()
}

fn response_snapshot(
    id: &str,
    created_at: u64,
    status: &str,
    model: &str,
    output: Vec<Value>,
    error: Option<String>,
) -> Value {
    json!({
        "id": id,
        "object": "response",
        "created_at": created_at,
        "status": status,
        "model": model,
        "output": output,
        "usage": {
            "input_tokens": 0,
            "output_tokens": 0,
            "total_tokens": 0,
        },
        "error": error.map(|message| json!({"message": message, "type": "browser_error"})),
    })
}

async fn send_output_started(
    tx: &mpsc::Sender<Result<Bytes, Infallible>>,
    item_id: &str,
    sequence: &mut u64,
) -> bool {
    let added_item = json!({
        "type": "message",
        "id": item_id,
        "status": "in_progress",
        "role": "assistant",
        "phase": "final_answer",
        "content": [],
    });
    if !send_event(
        tx,
        "response.output_item.added",
        json!({"output_index": 0, "item": added_item}),
        sequence,
    )
    .await
    {
        return false;
    }
    send_event(
        tx,
        "response.content_part.added",
        json!({
            "item_id": item_id,
            "output_index": 0,
            "content_index": 0,
            "part": {"type": "output_text", "text": "", "annotations": []}
        }),
        sequence,
    )
    .await
}

async fn send_output_delta(
    tx: &mpsc::Sender<Result<Bytes, Infallible>>,
    item_id: &str,
    delta: &str,
    sequence: &mut u64,
) -> bool {
    send_event(
        tx,
        "response.output_text.delta",
        json!({
            "item_id": item_id,
            "output_index": 0,
            "content_index": 0,
            "delta": delta,
        }),
        sequence,
    )
    .await
}

async fn send_event(
    tx: &mpsc::Sender<Result<Bytes, Infallible>>,
    name: &str,
    mut data: Value,
    sequence: &mut u64,
) -> bool {
    if let Some(object) = data.as_object_mut() {
        object.insert("type".into(), json!(name));
        object.insert("sequence_number".into(), json!(*sequence));
    }
    *sequence = sequence.saturating_add(1);
    let frame = format!("event: {name}\ndata: {}\n\n", data);
    send_raw(tx, &frame).await
}

async fn send_raw(tx: &mpsc::Sender<Result<Bytes, Infallible>>, frame: &str) -> bool {
    tx.send(Ok(Bytes::copy_from_slice(frame.as_bytes())))
        .await
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_and_legacy_requests_map_to_efforts() {
        let request = |model: &str, effort: Option<&str>| {
            let mut value = serde_json::json!({ "model": model });
            if let Some(effort) = effort {
                value["reasoning"] = serde_json::json!({ "effort": effort });
            }
            value
        };
        assert_eq!(effort_index(&request("mnelyra-web/low", None)).unwrap(), 0);
        // Keep the old slug usable for already-open Codex threads/configs.
        assert_eq!(
            effort_index(&request("mnelyra-web/instant", None)).unwrap(),
            0
        );
        assert_eq!(
            effort_index(&request("mnelyra-web/medium", None)).unwrap(),
            1
        );
        assert_eq!(effort_index(&request("mnelyra-web/high", None)).unwrap(), 2);
        assert_eq!(
            effort_index(&request("gpt-5.6-sol", Some("low"))).unwrap(),
            0
        );
        assert_eq!(
            effort_index(&request("gpt-5.6-sol", Some("medium"))).unwrap(),
            1
        );
        assert_eq!(
            effort_index(&request("gpt-5.6-sol", Some("high"))).unwrap(),
            2
        );
        assert!(effort_index(&request("gpt-5.6-sol", Some("ultra"))).is_err());
        assert!(effort_index(&request("mnelyra-web/unknown", None)).is_err());
    }

    #[test]
    fn prompt_chunking_preserves_unicode_exactly() {
        let source = "甲🙂乙".repeat(20_000);
        let chunks = prompt_chunks(&source, 16_000);
        assert_eq!(chunks.concat(), source);
        assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 16_000));
    }

    #[test]
    fn rendered_block_normalization_matches_markdown_join_contract() {
        let blocks = vec![
            "\n\n第一段\n\n".to_string(),
            "\n\n第二段🙂\n\n\n".to_string(),
            "第三段".to_string(),
        ];
        assert_eq!(
            normalize_rendered_blocks(&blocks),
            "第一段\n\n第二段🙂\n\n第三段"
        );
    }

    #[test]
    fn stable_stream_prefix_is_unicode_safe() {
        let previous = format!("{}旧尾巴", "甲🙂乙".repeat(80));
        let current = format!("{}新尾巴", "甲🙂乙".repeat(80));
        let common = common_prefix_byte_len(&previous, &current);
        assert!(previous.is_char_boundary(common));
        assert!(current.is_char_boundary(common));

        let end = stable_stream_end(&current, common, 24);
        assert!(current.is_char_boundary(end));
        assert!(end < common);
        assert!(current[..end].chars().count() > 100);
    }

    #[test]
    fn long_single_prose_block_can_stream_before_completion() {
        let previous = format!("{}正在继续", "这是一段稳定正文。".repeat(40));
        let current = format!("{}而且还没有结束", "这是一段稳定正文。".repeat(40));

        let common = common_prefix_byte_len(&previous, &current);
        let end = stable_stream_end(&current, common, STREAM_TEXT_TAIL_HOLD_CHARS);
        assert!(
            end > 0,
            "a long single prose block must expose a stable prefix"
        );
        assert!(end < common, "the active tail must remain buffered");
        assert!(current.is_char_boundary(end));
    }

    #[test]
    fn structured_markdown_can_stream_a_stable_prefix() {
        let previous = format!("# 标题\n\n- {}\n- 仍在生成", "稳定列表正文。".repeat(30));
        let current = format!("# 标题\n\n- {}\n- 继续生成中", "稳定列表正文。".repeat(30));
        let common = common_prefix_byte_len(&previous, &current);
        let end = stable_stream_end(&current, common, STREAM_TEXT_TAIL_HOLD_CHARS);
        assert!(
            end > 0,
            "structured Markdown must expose a stable prefix too"
        );
        assert!(
            end < common,
            "the active Markdown tail must remain buffered"
        );
        assert!(current.is_char_boundary(end));
    }

    #[test]
    fn prompt_contract_contains_context_without_tools_surface() {
        let request = json!({
            "model": "mnelyra-web/high",
            "instructions": "system",
            "input": [{"role":"user","content":"hello"}],
            "tools": [{"type":"function","name":"danger"}],
        });
        let prompt = compile_prompt(&request).unwrap();
        assert!(prompt.contains("<mnelyra_codex_context>"));
        assert!(prompt.contains("hello"));
        assert!(!prompt.contains("\"danger\""));
    }
}
