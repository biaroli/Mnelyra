import { invoke } from "@tauri-apps/api/core";
import type {
  ActiveWorkspaceState,
  ActivitySnapshot,
  SwitchCheck,
} from "$lib/types";

export async function getActiveWorkspaceState(): Promise<ActiveWorkspaceState> {
  return invoke<ActiveWorkspaceState>("get_active_workspace_state");
}

export async function getWorkspaceActivity(id: string): Promise<ActivitySnapshot> {
  return invoke<ActivitySnapshot>("get_workspace_activity", { id });
}

export async function canSwitchWorkspace(id: string): Promise<SwitchCheck> {
  return invoke<SwitchCheck>("can_switch_workspace", { id });
}

export async function activateWorkspace(id: string): Promise<ActiveWorkspaceState> {
  return invoke<ActiveWorkspaceState>("activate_workspace", { id });
}
