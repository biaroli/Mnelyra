import { invoke } from "@tauri-apps/api/core";
import type { WebModelBridgeStatus, OpenAiConnectorSettings } from "$lib/types";

export async function getOpenAiConnectorSettings(): Promise<OpenAiConnectorSettings> {
  return invoke<OpenAiConnectorSettings>("get_openai_connector_settings");
}

export async function saveOpenAiConnectorSettings(input: {
  tunnelId: string;
  alias: string;
  runtimeApiKey?: string | null;
}): Promise<OpenAiConnectorSettings> {
  return invoke<OpenAiConnectorSettings>("save_openai_connector_settings", {
    tunnelId: input.tunnelId,
    alias: input.alias,
    runtimeApiKey: input.runtimeApiKey ?? null,
  });
}

export async function getWebModelBridgeStatus(): Promise<WebModelBridgeStatus> {
  return invoke<WebModelBridgeStatus>("get_web_model_bridge_status");
}

export async function startWebModelBridge(): Promise<WebModelBridgeStatus> {
  return invoke<WebModelBridgeStatus>("start_web_model_bridge");
}

export async function stopWebModelBridge(): Promise<WebModelBridgeStatus> {
  return invoke<WebModelBridgeStatus>("stop_web_model_bridge");
}
