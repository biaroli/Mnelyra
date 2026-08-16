import { invoke } from "@tauri-apps/api/core";

export type SharedSecretKey =
  | "oauth_client_id"
  | "oauth_approval_code"
  | "bearer_token"
  | "cloudflare_token"
  | "frp_token";

export async function getSharedSecret(key: SharedSecretKey): Promise<string | null> {
  return invoke<string | null>("get_shared_secret", { key });
}

export async function setSharedSecret(key: SharedSecretKey, value: string): Promise<void> {
  return invoke("set_shared_secret", { key, value });
}

export async function regenerateSharedSecret(key: SharedSecretKey): Promise<string> {
  return invoke<string>("regenerate_shared_secret", { key });
}
