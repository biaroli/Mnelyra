import { invoke } from "@tauri-apps/api/core";

export async function startBackgroundServices(): Promise<boolean> {
  return invoke<boolean>("start_background_services");
}
