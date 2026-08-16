import { writable } from "svelte/store";
import type {
  ActiveWorkspaceState,
  ActivitySnapshot,
  ProviderStatus,
  RuntimeState,
  WorkspaceProfile,
} from "$lib/types";

export const workspaces = writable<WorkspaceProfile[]>([]);
export const mcpRuntimeStates = writable<Record<string, RuntimeState>>({});

export const activeWorkspaceState = writable<ActiveWorkspaceState>({
  workspaceId: null,
  phase: "none",
  generation: 0,
  sinceUnixMs: null,
  message: null,
});

export const workspaceActivity = writable<Record<string, ActivitySnapshot>>({});

export const providerStatuses = writable<Record<string, ProviderStatus>>({});

/** @deprecated use mcpRuntimeStates */
export const runtimeStates = mcpRuntimeStates;
