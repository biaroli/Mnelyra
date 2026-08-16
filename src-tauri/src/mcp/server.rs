use std::sync::Arc;

use serde_json::Value;

use crate::tools::{
    call_tool, list_tools_for_profile, wrap_mcp_tool_result, SharedToolContext, ToolContext,
    Workspace,
};
use crate::workspace::AuthConfig;

pub type SharedState = SharedToolContext;

pub fn handle_request(state: &SharedState, body: &Value) -> Value {
    let method = body.get("method").and_then(Value::as_str).unwrap_or("");
    let id = body.get("id").cloned().unwrap_or(Value::Null);
    let params = body.get("params").cloned().unwrap_or(Value::Null);

    if id.is_null() && method.starts_with("notifications/") {
        return Value::Null;
    }

    let result = match method {
        "initialize" => Ok(initialize_result()),
        "ping" => Ok(serde_json::json!({})),
        "tools/list" => {
            let mut tools = list_tools_for_profile(&state.tool_profile);
            tools.retain(|tool| {
                tool.get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| {
                        crate::tools::policy::ceiling_allows_tool(
                            &state.policy.permission_ceiling,
                            name,
                        )
                    })
            });
            tools.extend(codex_control_tools().into_iter().filter(|tool| {
                state.policy.permission_ceiling != "read_only"
                    || tool["annotations"]["readOnlyHint"] == Value::Bool(true)
            }));
            Ok(serde_json::json!({ "tools": tools }))
        }
        "tools/call" => handle_tools_call(state, &params),
        _ => Err(serde_json::json!({
            "code": -32601,
            "message": format!("Method not found: {method}")
        })),
    };

    match result {
        Ok(result) => serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(error) => serde_json::json!({ "jsonrpc": "2.0", "id": id, "error": error }),
    }
}

fn codex_control_tools() -> Vec<Value> {
    vec![
        mcp_tool(
            "codex_start_task",
            "Start Codex task",
            "Start a Mnelyra-managed native Codex task bound to the active workspace.",
            serde_json::json!({
                "type": "object",
                "required": ["title", "prompt"],
                "properties": {
                    "title": { "type": "string", "minLength": 1 },
                    "prompt": { "type": "string", "minLength": 1 }
                },
                "additionalProperties": false
            }),
            false,
        ),
        mcp_tool(
            "codex_list_sessions",
            "List Codex sessions",
            "List Mnelyra Codex sessions bound to the active workspace.",
            serde_json::json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            true,
        ),
        mcp_tool(
            "codex_send_input",
            "Send Codex input",
            "Continue a Mnelyra Codex session with additional input.",
            session_string_schema("input"),
            false,
        ),
        mcp_tool(
            "codex_cancel_session",
            "Cancel Codex session",
            "Interrupt the active turn for a Mnelyra Codex session.",
            session_only_schema(),
            false,
        ),
        mcp_tool(
            "codex_compact_session",
            "Compact Codex session",
            "Manually compact an idle Mnelyra Codex session. Use only as a recovery/maintenance action; native automatic compaction remains the default.",
            session_only_schema(),
            false,
        ),
        mcp_tool(
            "codex_session_events",
            "Read Codex session events",
            "Read incremental Mnelyra Codex session events using a revision cursor.",
            serde_json::json!({
                "type": "object",
                "required": ["session_id"],
                "properties": {
                    "session_id": { "type": "string", "minLength": 1 },
                    "cursor": { "type": "integer", "minimum": 0, "default": 0 },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 240, "default": 160 }
                },
                "additionalProperties": false
            }),
            true,
        ),
        mcp_tool(
            "codex_pending_requests",
            "Read Codex approvals",
            "List pending Codex approval/input requests for a Mnelyra session.",
            session_only_schema(),
            true,
        ),
        mcp_tool(
            "codex_respond_request",
            "Respond to Codex approval",
            "Respond to a pending Codex approval or input request.",
            serde_json::json!({
                "type": "object",
                "required": ["session_id", "request_id", "action"],
                "properties": {
                    "session_id": { "type": "string", "minLength": 1 },
                    "request_id": { "type": "string", "minLength": 1 },
                    "action": { "type": "string", "minLength": 1 }
                },
                "additionalProperties": false
            }),
            false,
        ),
    ]
}

fn mcp_tool(
    name: &str,
    title: &str,
    description: &str,
    input_schema: Value,
    read_only: bool,
) -> Value {
    serde_json::json!({
        "name": name,
        "title": title,
        "description": description,
        "inputSchema": input_schema,
        "annotations": {
            "title": title,
            "readOnlyHint": read_only,
            "destructiveHint": false,
            "idempotentHint": read_only,
            "openWorldHint": false
        }
    })
}

fn session_only_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "required": ["session_id"],
        "properties": { "session_id": { "type": "string", "minLength": 1 } },
        "additionalProperties": false
    })
}

fn session_string_schema(field: &str) -> Value {
    let mut properties = serde_json::Map::new();
    properties.insert(
        "session_id".into(),
        serde_json::json!({ "type": "string", "minLength": 1 }),
    );
    properties.insert(
        field.to_string(),
        serde_json::json!({ "type": "string", "minLength": 1 }),
    );
    serde_json::json!({
        "type": "object",
        "required": ["session_id", field],
        "properties": properties,
        "additionalProperties": false
    })
}

fn initialize_result() -> Value {
    serde_json::json!({
        "protocolVersion": "2025-06-18",
        "capabilities": {
            "tools": { "listChanged": false },
            "logging": {}
        },
        "serverInfo": {
            "name": "mnelyra",
            "title": "Mnelyra",
            "version": env!("CARGO_PKG_VERSION")
        },
        "instructions": "Use these tools only for local coding operations inside the configured workspace. At the start of every new client conversation, before answering the user's first request, call history_session_bootstrap exactly once and pass the user's verbatim first request as initial_user_input. Treat bootstrap as required conversation initialization: it creates or resumes a lossless Markdown archive and returns bounded current state, not all history. Use history_session_search followed by history_session_read only when exact earlier context is needed. history_session_read returns a bounded UTF-8-safe page; follow next_cursor with the returned content hash until the relevant archive is complete. Repeated successful bootstrap calls in the same conversation resume the same session and must not create duplicates. Preserve session_key and current_path returned by bootstrap, then pass them unchanged as session_key and expected_path to every history_session_checkpoint call. After completing each user-requested task in the conversation, call history_session_checkpoint before the final response and pass that user's verbatim request as raw_user_input. Only state that progress was saved after checkpoint returns ok=true with the same session_key and path. The server cannot access client transcript text that was not provided as a tool argument; persistence is not automatic background persistence."
    })
}

fn handle_tools_call(state: &SharedState, params: &Value) -> Result<Value, Value> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| serde_json::json!({ "code": -32602, "message": "Missing tool name" }))?;
    let args = tool_arguments(name, params);

    let canonical_name = crate::tools::registry::canonical_tool_name(name);
    let known = crate::tools::registry::exposed_tool_names(&state.tool_profile);
    if !known.iter().any(|n| n == &canonical_name) {
        return Err(serde_json::json!({
            "code": -32602,
            "message": format!("Unknown tool: {name}"),
            "data": { "reason": "unknown_tool" }
        }));
    }

    let structured = call_tool(state.as_ref(), canonical_name, &args);
    Ok(wrap_mcp_tool_result(canonical_name, &args, structured))
}

fn tool_arguments(name: &str, params: &Value) -> Value {
    let mut args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    if name.starts_with("history_session_") {
        if let Some(session_key) = params
            .get("_meta")
            .and_then(|meta| meta.get("openai/session"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if !args.is_object() {
                args = serde_json::json!({});
            }
            args["_host_session_key"] = Value::String(session_key.to_string());
        }
    }
    args
}

pub fn new_state(
    workspace: Workspace,
    workspace_id: String,
    activity: crate::activity::ActivityCoordinator,
    auth: AuthConfig,
    policy: crate::tools::policy::PolicySettings,
    tool_profile: String,
    permission_mode: String,
) -> SharedState {
    Arc::new(ToolContext::from_workspace_scoped(
        workspace,
        auth,
        policy,
        tool_profile,
        permission_mode,
        workspace_id,
        activity,
    ))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;

    use serde_json::{json, Value};

    use crate::tools::ToolContext;

    use super::{codex_control_tools, handle_request, initialize_result, tool_arguments};

    #[test]
    fn initialize_instructions_define_the_history_persistence_workflow() {
        let initialized = initialize_result();
        let instructions = initialized["instructions"].as_str().expect("instructions");
        assert!(instructions.contains("history_session_bootstrap"));
        assert!(instructions.contains("At the start of every new client conversation"));
        assert!(instructions.contains("before answering the user's first request"));
        assert!(instructions.contains("required conversation initialization"));
        assert!(instructions.contains("initial_user_input"));
        assert!(instructions.contains("must not create duplicates"));
        assert!(instructions.contains("history_session_checkpoint"));
        assert!(instructions.contains("raw_user_input"));
        assert!(instructions.contains("history_session_search"));
        assert!(instructions.contains("history_session_read"));
        assert!(instructions.contains("follow next_cursor"));
        assert!(instructions.contains("session_key and current_path returned by bootstrap"));
        assert!(instructions.contains("session_key and expected_path"));
        assert!(instructions.contains("After completing each user-requested task"));
        assert!(instructions.contains("before the final response"));
        assert!(instructions.contains("checkpoint returns ok=true"));
        assert!(instructions.contains("not automatic background persistence"));
    }

    #[test]
    fn initialize_does_not_claim_tool_catalog_notifications_without_a_stream() {
        let initialized = initialize_result();

        assert_eq!(initialized["capabilities"]["tools"]["listChanged"], false);
    }

    #[test]
    fn codex_control_catalog_is_stable_for_remote_clients() {
        let tools = codex_control_tools();
        let names = tools
            .iter()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str))
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "codex_start_task",
                "codex_list_sessions",
                "codex_send_input",
                "codex_cancel_session",
                "codex_compact_session",
                "codex_session_events",
                "codex_pending_requests",
                "codex_respond_request",
            ]
        );
        let send = tools
            .iter()
            .find(|tool| tool["name"] == "codex_send_input")
            .expect("send tool");
        assert!(send["inputSchema"]["properties"].get("input").is_some());
        assert!(send["inputSchema"]["properties"].get("field").is_none());
    }

    #[test]
    fn chatgpt_session_metadata_is_injected_only_for_history_tools() {
        let params = json!({
            "arguments": {"session_key": "explicit"},
            "_meta": {"openai/session": "chatgpt-conversation"}
        });
        let history = tool_arguments("history_session_bootstrap", &params);
        assert_eq!(history["session_key"], "explicit");
        assert_eq!(history["_host_session_key"], "chatgpt-conversation");

        let existing = tool_arguments("read_file", &params);
        assert_eq!(existing["session_key"], "explicit");
        assert!(existing.get("_host_session_key").is_none());
    }

    #[test]
    fn host_session_key_takes_precedence_over_explicit_session_key() {
        let workspace = tempfile::tempdir().expect("workspace tempdir");
        let harness = tempfile::tempdir().expect("harness tempdir");
        let state = Arc::new(
            ToolContext::for_test(workspace.path().to_path_buf(), harness.path().to_path_buf())
                .expect("tool context"),
        );
        let response = handle_request(
            &state,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "history_session_bootstrap",
                    "arguments": {
                        "session_key": "explicit-session",
                        "initial_user_input": "保存首轮原文"
                    },
                    "_meta": {"openai/session": "chatgpt-session"}
                }
            }),
        );
        let structured = &response["result"]["structuredContent"];
        assert_eq!(structured["ok"], true);
        assert_eq!(structured["session_key_source"], "platform_conversation_id");
        assert_eq!(structured["session_key"], "chatgpt-session");
        assert_eq!(structured["initial_input_captured"], true);
        let content = fs::read_to_string(workspace.path().join(".mnelyra/history-session/1.md"))
            .expect("read history file");
        assert!(content.contains("**Session key:** chatgpt-session"));
        assert!(!content.contains("**Session key:** explicit-session"));
    }

    #[test]
    fn legacy_grep_calls_are_mapped_to_the_public_grep_text_tool() {
        let workspace = tempfile::tempdir().expect("workspace tempdir");
        let harness = tempfile::tempdir().expect("harness tempdir");
        fs::write(workspace.path().join("sample.txt"), "catalog needle")
            .expect("write sample file");
        let state = Arc::new(
            ToolContext::for_test(workspace.path().to_path_buf(), harness.path().to_path_buf())
                .expect("tool context"),
        );

        let response = handle_request(
            &state,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "grep",
                    "arguments": {"query": "needle", "path": "."}
                }
            }),
        );

        assert!(response.get("error").is_none());
        assert_eq!(response["result"]["structuredContent"]["ok"], true);
    }
}
