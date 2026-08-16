import { invoke } from "@tauri-apps/api/core";
import type {
  PendingSessionRequest,
  CodexConfigReadResponse,
  ProviderDescriptor,
  ProviderStatus,
  ProviderCheckpoint,
  SessionEvent,
  SessionEventPage,
  TaskSession,
  WorkspaceMemoryOverview,
} from "$lib/types";

export async function listProviders(): Promise<ProviderDescriptor[]> {
  return invoke<ProviderDescriptor[]>("list_providers");
}

export async function getProviderStatus(id: string): Promise<ProviderStatus> {
  return invoke<ProviderStatus>("get_provider_status", { id });
}

export async function listSessions(): Promise<TaskSession[]> {
  return invoke<TaskSession[]>("list_sessions");
}

export async function startProviderTask(
  workspaceId: string,
  title: string,
  prompt: string,
): Promise<TaskSession> {
  return invoke<TaskSession>("start_provider_task", { workspaceId, title, prompt });
}

export async function sendSessionInput(sessionId: string, input: string): Promise<TaskSession> {
  return invoke<TaskSession>("send_session_input", { sessionId, input });
}

export async function cancelSession(sessionId: string): Promise<TaskSession> {
  return invoke<TaskSession>("cancel_session", { sessionId });
}

export async function compactSession(sessionId: string): Promise<TaskSession> {
  return invoke<TaskSession>("compact_session", { sessionId });
}

export async function getCodexContextPolicy(): Promise<CodexConfigReadResponse> {
  return invoke<CodexConfigReadResponse>("get_codex_context_policy");
}

export async function setCodexAutoCompactLimit(
  tokenLimit: number | null,
): Promise<Record<string, unknown>> {
  return invoke<Record<string, unknown>>("set_codex_auto_compact_limit", {
    tokenLimit,
  });
}

export async function setPermissionCeiling(
  mode: "automatic" | "read_only" | "custom",
): Promise<void> {
  return invoke<void>("set_permission_ceiling", { mode });
}

export async function getSessionEvents(sessionId: string): Promise<SessionEvent[]> {
  return invoke<SessionEvent[]>("get_session_events", { sessionId });
}

export async function getSessionEventPage(
  sessionId: string,
  cursor: number,
  limit = 160,
): Promise<SessionEventPage> {
  return invoke<SessionEventPage>("get_session_event_page", { sessionId, cursor, limit });
}

export async function getPendingSessionRequests(
  sessionId: string,
): Promise<PendingSessionRequest[]> {
  return invoke<PendingSessionRequest[]>("get_pending_session_requests", { sessionId });
}

export async function respondSessionRequest(
  sessionId: string,
  requestId: string,
  action: "accept" | "accept_for_session" | "decline" | "cancel",
): Promise<void> {
  return invoke<void>("respond_session_request", { sessionId, requestId, action });
}

export async function getWorkspaceMemoryOverview(
  workspaceId: string,
): Promise<WorkspaceMemoryOverview> {
  return invoke<WorkspaceMemoryOverview>("get_workspace_memory_overview", { workspaceId });
}

export async function getProviderCheckpoint(
  workspaceId: string,
  checkpointId: string,
): Promise<ProviderCheckpoint> {
  return invoke<ProviderCheckpoint>("get_provider_checkpoint", { workspaceId, checkpointId });
}
