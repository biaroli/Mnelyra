export type RuntimeState = "stopped" | "starting" | "running" | "stopping" | "error";

export type ActiveWorkspacePhase =
  | "none"
  | "activating"
  | "active"
  | "draining"
  | "switching"
  | "error";

export interface ActiveWorkspaceState {
  workspaceId: string | null;
  phase: ActiveWorkspacePhase;
  generation: number;
  sinceUnixMs: number | null;
  message: string | null;
}

export interface ProviderCheckpointSummary {
  checkpointId: string;
  providerId: string;
  mnelyraSessionId: string;
  providerSessionId: string;
  providerTurnId: string | null;
  capturedAt: string;
  contentSha256: string;
}

export interface WorkspaceMemoryOverview {
  workspaceId: string;
  historyRoot: string;
  manifestExists: boolean;
  stateExists: boolean;
  archiveRevision: string;
  memoryRevision: string;
  generatedAt: string;
  currentFocus: string;
  recentChanges: string[];
  openItems: string[];
  providerCheckpointCount: number;
  providerCheckpoints: ProviderCheckpointSummary[];
}

export interface ProviderCheckpoint {
  version: number;
  checkpointId: string;
  providerId: string;
  mnelyraSessionId: string;
  workspaceId: string;
  canonicalWorkspacePath: string;
  providerSessionId: string;
  providerTurnId: string | null;
  capturedAt: string;
  source: string;
  contentSha256: string;
  threadMetadata: unknown;
  turnSnapshot: unknown;
}

export interface OpenAiConnectorSettings {
  enabled: boolean;
  tunnelId: string;
  alias: string;
  hasRuntimeKey: boolean;
  tunnelClientVersion: string;
}

export interface OpenAiConnectorStatus {
  configured: boolean;
  hasRuntimeKey: boolean;
  tunnelId: string;
  alias: string;
  binaryInstalled: boolean;
  binaryVersion: string;
  processRunning: boolean;
  healthy: boolean;
  ready: boolean;
  runtimeState: string | null;
  uiUrl: string | null;
  detail: string;
}

export type ProviderCapability =
  | "status"
  | "sessions"
  | "start_task"
  | "send_input"
  | "cancel_task"
  | "compaction"
  | "drain"
  | "resume";

export interface CodexConfigReadResponse {
  config?: Record<string, unknown>;
  origins?: Record<string, unknown>;
  layers?: unknown[];
}

export type ProviderState =
  | "unavailable"
  | "ready"
  | "busy"
  | "version_mismatch"
  | "error";

export interface ProviderDescriptor {
  id: string;
  name: string;
  capabilities: ProviderCapability[];
}

export interface ProviderStatus {
  providerId: string;
  state: ProviderState;
  configured: boolean;
  activityKnown: boolean;
  version: string | null;
  mode: string | null;
  pid: number | null;
  acceptingTasks: boolean | null;
  activeTurns: number;
  activeHttpTurns: number;
  activeBrowserTurns: number;
  sessionReady: boolean | null;
  message: string;
}

export type TaskSessionState =
  | "queued"
  | "starting"
  | "running"
  | "waiting_for_user"
  | "waiting_for_tool"
  | "compacting"
  | "draining"
  | "completed"
  | "cancelled"
  | "failed";

export interface TaskSession {
  id: string;
  workspaceId: string;
  canonicalWorkspacePath: string;
  providerId: string;
  providerSessionId: string | null;
  title: string;
  state: TaskSessionState;
  createdAt: string;
  updatedAt: string;
  lastActivityAt: string;
}

export interface SessionEvent {
  id: string;
  sessionId: string;
  kind: string;
  text: string;
  createdAt: string;
  details: unknown | null;
  revision: number;
}

export interface SessionEventPage {
  events: SessionEvent[];
  nextCursor: number;
  reset: boolean;
}

export interface PendingSessionRequest {
  id: string;
  sessionId: string;
  method: string;
  params: unknown;
  createdAt: string;
}

export interface ActivitySnapshot {
  workspaceId: string;
  activeMcpRequests: number;
  runningExecSessions: number;
  activeProviderTurns: number;
  pendingProviderOperations: number;
  providerActivityKnown: boolean;
  drainRequested: boolean;
}

export interface SwitchCheck {
  allowed: boolean;
  reasons: string[];
  activity: ActivitySnapshot;
}

export const DEFAULT_SERVICE_PORT = 28766;

export interface TunnelConfig {
  type: string;
  public_url: string;
  frp_server: string;
  frp_subdomain: string;
  frp_profile_id?: string;
  frp_server_port?: number;
  cloudflare_mode: string;
}

export function workspaceRootName(path: string): string {
  const cleaned = path.trim().replace(/[\\/]+$/, "");
  if (!cleaned) return "工作区";
  const parts = cleaned.split(/[\\/]/).filter(Boolean);
  return parts.at(-1) ?? "工作区";
}

export interface AuthConfig {
  type: string;
  oauth_client_id: string;
  use_shared_secrets?: boolean;
}

export interface GlobalAuthConfig {
  mcpAuthType: string;
}

export interface GlobalGeneralConfig {
  configured: boolean;
  permissionCeiling: "automatic" | "read_only" | "custom";
  mcpTunnel: TunnelConfig;
  mcpRuntime: RuntimeConfig;
}

export interface RuntimeConfig {
  local_port: number;
  tool_profile: string;
  permission_mode: string;
  runtime_command?: string;
  allowed_commands?: string;
  workspace_local_entries?: boolean;
  workspace_script_extensions?: string;
}

export interface WorkspaceProfile {
  id: string;
  name: string;
  path: string;
  tunnel: TunnelConfig;
  auth: AuthConfig;
  runtime: RuntimeConfig;
}

export interface RuntimeStatus {
  state: RuntimeState;
  pid: number | null;
  localMessage: string;
  publicMessage: string;
  localEndpoint: string;
  publicEndpoint: string;
}

export function mcpLocalEndpoint(port: number): string {
  return `http://127.0.0.1:${port}/mcp`;
}

export interface FrpProfileSummary {
  id: string;
  name: string;
  server: string;
  serverPort: number;
}

export function frpPublicUrl(
  tunnelType: string,
  frpSubdomain: string,
  frpServer: string,
  frpProfileId: string | undefined,
  profiles: FrpProfileSummary[],
  publicUrl = "",
): string {
  if (tunnelType !== "frp" || !frpSubdomain) {
    return publicUrl.replace(/\/$/, "");
  }
  const server =
    profiles.find((profile) => profile.id === frpProfileId)?.server ?? frpServer;
  if (!server) return publicUrl.replace(/\/$/, "");
  return `https://${frpSubdomain}.${server}`;
}
