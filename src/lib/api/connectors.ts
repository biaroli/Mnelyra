import { invoke } from "@tauri-apps/api/core";
import type { CodexWebBridgeStatus, OpenAiConnectorSettings } from "$lib/types";

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

export async function getCodexWebBridgeStatus(): Promise<CodexWebBridgeStatus> {
  return invoke<CodexWebBridgeStatus>("get_codex_web_bridge_status");
}

export async function startCodexWebBridge(): Promise<CodexWebBridgeStatus> {
  return invoke<CodexWebBridgeStatus>("start_codex_web_bridge");
}
