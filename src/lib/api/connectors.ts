import { invoke } from "@tauri-apps/api/core";
import type { OpenAiConnectorSettings, OpenAiConnectorStatus } from "$lib/types";

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

export async function installOpenAiTunnelClient(): Promise<OpenAiConnectorStatus> {
  return invoke<OpenAiConnectorStatus>("install_openai_tunnel_client");
}

export async function getOpenAiConnectorStatus(): Promise<OpenAiConnectorStatus> {
  return invoke<OpenAiConnectorStatus>("get_openai_connector_status");
}

export async function startOpenAiConnector(): Promise<OpenAiConnectorStatus> {
  return invoke<OpenAiConnectorStatus>("start_openai_connector");
}

export async function stopOpenAiConnector(): Promise<OpenAiConnectorStatus> {
  return invoke<OpenAiConnectorStatus>("stop_openai_connector");
}
