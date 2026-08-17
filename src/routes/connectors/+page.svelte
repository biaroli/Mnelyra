<script lang="ts">
  import { onMount } from "svelte";
  import { openUrl } from "$lib/api/app-info";
  import {
    getCodexWebBridgeStatus,
    getOpenAiConnectorSettings,
    saveOpenAiConnectorSettings,
    startCodexWebBridge,
  } from "$lib/api/connectors";
  import { showToast } from "$lib/stores/toast";
  import { uiLocale } from "$lib/stores/locale";
  import { activeWorkspaceState, mcpRuntimeStates } from "$lib/stores/app";
  import { getRuntimeStatus } from "$lib/api/workspaces";
  import ConnectionRoutingPanel from "$lib/components/ConnectionRoutingPanel.svelte";
  import type { CodexWebBridgeStatus, OpenAiConnectorSettings } from "$lib/types";
  import { ExternalLink, Play, Save } from "@lucide/svelte";

  let settings = $state<OpenAiConnectorSettings | null>(null);
  let codexWeb = $state<CodexWebBridgeStatus | null>(null);
  let tunnelId = $state("");
  let runtimeApiKey = $state("");
  let publicRouteReady = $state(false);
  let busy = $state(false);
  let timer: number | undefined;
  const zh = $derived($uiLocale === "zh-CN");
  const mcpReady = $derived(
    $activeWorkspaceState.workspaceId != null
      && $mcpRuntimeStates[$activeWorkspaceState.workspaceId] === "running",
  );
  const connectionFlowing = $derived(mcpReady && publicRouteReady);
  const codexTunnelRequired = $derived(codexWeb?.mode === "full");

  async function refresh() {
    try {
      const workspaceId = $activeWorkspaceState.workspaceId;
      const [nextSettings, nextCodexWeb, runtime] = await Promise.all([
        getOpenAiConnectorSettings(),
        getCodexWebBridgeStatus().catch(() => null),
        workspaceId ? getRuntimeStatus(workspaceId).catch(() => null) : Promise.resolve(null),
      ]);
      settings = nextSettings;
      codexWeb = nextCodexWeb;
      publicRouteReady = Boolean(runtime?.state === "running" && runtime.publicEndpoint?.trim());
      if (!tunnelId) tunnelId = nextCodexWeb?.tunnelId || nextSettings.tunnelId;
    } catch (error) {
      showToast(String(error), { title: zh ? "连接状态读取失败" : "Failed to read connector status", kind: "error", duration: 8000 });
    }
  }

  async function recoverCodexWeb() {
    if (busy) return;
    busy = true;
    const installingRoute = !codexWeb?.routeInstalled;
    try {
      if (tunnelId.trim() || runtimeApiKey.trim()) await persistTunnelSetup();
      codexWeb = await startCodexWebBridge();
      showToast(installingRoute
        ? (zh ? "网页模型路由已安装。重启 Codex 一次以刷新原生模型列表。" : "Web-model routing is installed. Restart Codex once to refresh its native model catalog.")
        : (zh ? "网页模型接入已恢复。" : "Web model access is ready."), {
        title: installingRoute ? (zh ? "已安装到 Codex" : "Installed in Codex") : (zh ? "网页模型已就绪" : "Web models ready"),
        kind: "success",
      });
    } catch (error) {
      showToast(String(error), { title: zh ? "网页模型恢复失败" : "Failed to recover web models", kind: "error", duration: 11000 });
      codexWeb = await getCodexWebBridgeStatus().catch(() => codexWeb);
    } finally {
      busy = false;
    }
  }

  async function persistTunnelSetup() {
    settings = await saveOpenAiConnectorSettings({
      tunnelId,
      alias: settings?.alias || "mnelyra",
      runtimeApiKey: runtimeApiKey.trim() || null,
    });
  }

  async function saveTunnelSetup() {
    if (busy) return;
    busy = true;
    try {
      await persistTunnelSetup();
      showToast(zh ? "配置已保存。OpenAI API Key 不写入命令行。" : "Settings saved. The OpenAI API Key is not written to the command line.", {
        title: zh ? "已保存" : "Saved",
        kind: "success",
      });
    } catch (error) {
      showToast(String(error), { title: zh ? "配置保存失败" : "Failed to save settings", kind: "error", duration: 9000 });
    } finally {
      busy = false;
    }
  }

  onMount(() => {
    void refresh();
    timer = window.setInterval(() => {
      void getCodexWebBridgeStatus().then((value) => (codexWeb = value)).catch(() => {});
      const workspaceId = $activeWorkspaceState.workspaceId;
      if (!workspaceId) {
        publicRouteReady = false;
        return;
      }
      void getRuntimeStatus(workspaceId)
        .then((runtime) => {
          publicRouteReady = Boolean(runtime.state === "running" && runtime.publicEndpoint?.trim());
        })
        .catch(() => {
          publicRouteReady = false;
        });
    // Web-model status runs the upstream route/doctor probes, which are substantially heavier
    // than the local tunnel status read. Five seconds keeps the UI fresh without spawning helper
    // processes several times per second while this page is open.
    }, 5000);
    return () => {
      if (timer) window.clearInterval(timer);
    };
  });
</script>

<section class="tx-page mn-connectors-page mn-console-surface">
  <header class="tx-page-header">
    <div>
      <p class="page-kicker">{zh ? "接入路径" : "CONNECTION PATHS"}</p>
      <h2>{zh ? "连接" : "Connections"}</h2>
    </div>
  </header>

  <div class="mn-connection-board">
    <div class="mn-connection-map" class:flowing={connectionFlowing} aria-label={zh ? "连接拓扑" : "Connection topology"}>
      <div class="mn-map-node client"><span>{zh ? "远程客户端" : "REMOTE CLIENTS"}</span><strong>ChatGPT · Claude · MCP</strong></div>
      <div class="mn-map-wire"><i></i><b></b></div>
      <div class="mn-map-node relay"><span>{zh ? "公网接入" : "PUBLIC ACCESS"}</span><strong>Cloudflare / FRP</strong></div>
      <div class="mn-map-wire"><i></i><b></b></div>
      <div class="mn-map-node core"><span>MNELYRA</span><strong>{zh ? "MCP 控制面" : "MCP control plane"}</strong></div>
      <div class="mn-map-wire"><i></i><b></b></div>
      <div class="mn-map-node workspace terminal"><strong>{zh ? "这台电脑" : "This computer"}</strong></div>
    </div>

    <div class="mn-connection-workbench">
      <ConnectionRoutingPanel />

      <section class="mn-connection-telemetry-panel mn-codex-bridge-panel" class:online={codexWeb?.ready}>
        <div class="mn-panel-cap">
          <div><span>MNELYRA</span><strong>{zh ? "网页模型接入" : "Web model bridge"}</strong></div>
          <b class:online={codexWeb?.ready}>
            {codexWeb?.ready ? (codexWeb.browserBusy ? (zh ? "任务中" : "BUSY") : (zh ? "已接入" : "READY")) : (codexWeb?.routeInstalled ? (zh ? "待恢复" : "RECOVER") : (zh ? "未接入" : "NOT CONNECTED"))}
          </b>
        </div>

        <div class="mn-codex-route-line" aria-label={zh ? "网页模型链路" : "Web model path"}>
          <span>CODEX</span><i>→</i><span>127.0.0.1:17841</span><i>→</i><span>CHATGPT WEB</span>
        </div>

        <div class="mn-codex-health-grid">
          <div class:ok={Boolean(codexWeb?.routeInstalled && codexWeb?.routeActive)}>
            <i></i><span>{zh ? "Codex 路由" : "Codex route"}</span><b>{codexWeb?.routeInstalled && codexWeb?.routeActive ? (zh ? "已接入" : "READY") : (zh ? "未接入" : "DOWN")}</b>
          </div>
          <div class:ok={Boolean(codexWeb?.proxyReady)}>
            <i></i><span>Responses</span><b>{codexWeb?.proxyReady ? "17841 OK" : "17841 DOWN"}</b>
          </div>
          <div class:ok={Boolean(codexWeb?.browserReady)}>
            <i></i><span>{zh ? "ChatGPT 浏览器" : "ChatGPT browser"}</span><b>{codexWeb?.browserBusy ? (zh ? "任务中" : "BUSY") : codexWeb?.browserReady ? (zh ? "已就绪" : "READY") : (zh ? "未就绪" : "DOWN")}</b>
          </div>
          <div class:ok={Boolean(!codexTunnelRequired || codexWeb?.tunnelReady)}>
            <i></i><span>OpenAI Tunnel</span><b>{!codexTunnelRequired ? (zh ? "无需" : "N/A") : codexWeb?.tunnelReady ? (zh ? "已就绪" : "READY") : (zh ? "未就绪" : "DOWN")}</b>
          </div>
        </div>

        <div class="mn-tunnel-live-row mn-codex-status-row">
          <div class="mn-tunnel-live-copy">
            <i class:ok={codexWeb?.ready}></i>
            <div>
              <strong>{zh ? "网页模型" : "Web models"}</strong>
              <span>
                {#if codexWeb?.ready}
                  {#if codexWeb.browserBusy}
                    {zh ? `正在运行 Codex 任务 · ${codexWeb.version || "runtime"}` : `Serving a Codex turn · ${codexWeb.version || "runtime"}`}
                  {:else}
                    {zh ? `已接入 Codex · ${codexWeb.version || "runtime"}` : `Connected to Codex · ${codexWeb.version || "runtime"}`}
                  {/if}
                {:else if codexWeb?.routeInstalled && codexWeb?.routeActive}
                  {zh ? "Codex 路由已安装，但网页模型运行时未就绪。" : "The Codex route is installed, but the Web-model runtime is not ready."}
                {:else if codexWeb?.cliInstalled}
                  {zh ? "网页模型运行时已找到，Codex 路由尚未就绪。" : "Web-model runtime found; Codex routing is not ready."}
                {:else}
                  {zh ? "未检测到网页模型运行时。" : "Web-model runtime was not detected."}
                {/if}
              </span>
              {#if codexWeb?.mode}
                <small>{codexWeb.mode.toUpperCase()}{codexWeb.appName ? ` · ${codexWeb.appName}` : ""}{codexWeb.tunnelId ? ` · ${codexWeb.tunnelId}` : ""}</small>
              {/if}
            </div>
          </div>
          {#if !codexWeb?.ready && codexWeb?.launcherInstalled && codexWeb?.cliInstalled}
            <button class="mn-mini-action primary" type="button" disabled={busy} onclick={() => void recoverCodexWeb()}><Play size={12} /> {codexWeb?.routeInstalled ? (zh ? "恢复" : "Recover") : (zh ? "安装到 Codex" : "Install in Codex")}</button>
          {/if}
        </div>

        <details class="mn-advanced-connection mn-codex-tunnel-setup">
          <summary>{zh ? "Full 模式 · OpenAI Tunnel 安装参数" : "Full mode · OpenAI Tunnel setup"}</summary>
          <p>{zh ? "Full 模式使用 OpenAI Tunnel 完成插件工具回路。这里的 API Key 只用于 Tunnel 认证，不用于模型 API 调用，不会产生模型 API token 消耗或模型 API credits 扣费。已有配置会直接复用。" : "Full mode uses OpenAI Tunnel for the plugin tool return path. The API key here is only used for Tunnel authentication, not for model API calls, so it does not consume model API tokens or model API credits. Existing setup is reused."}</p>
          <div class="mn-connector-form">
            <label>
              <span>Tunnel ID</span>
              <input bind:value={tunnelId} placeholder={codexWeb?.tunnelId || "tunnel_…"} disabled={busy} autocomplete="off" />
            </label>
            <label>
              <span>OpenAI API Key</span>
              <input type="password" bind:value={runtimeApiKey} placeholder={(codexWeb?.tunnelKeyConfigured || settings?.hasRuntimeKey) ? (zh ? "已配置 · 留空保持不变" : "configured · leave blank to keep") : (zh ? "粘贴 OpenAI API Key" : "paste an OpenAI API Key")} autocomplete="off" disabled={busy} />
            </label>
          </div>
          <div class="mn-connector-actions">
            <button class="mn-mini-action" type="button" disabled={busy} onclick={() => void saveTunnelSetup()}><Save size={12} /> {zh ? "保存" : "Save"}</button>
          </div>
          <div class="mn-connector-links">
            <button type="button" onclick={() => void openUrl("https://platform.openai.com/settings/organization/tunnels")}>OpenAI Tunnels <ExternalLink size={11} /></button>
            <button type="button" onclick={() => void openUrl("https://platform.openai.com/settings/organization/api-keys")}>OpenAI API Keys <ExternalLink size={11} /></button>
            <button type="button" onclick={() => void openUrl("https://chatgpt.com/#settings/Connectors")}>ChatGPT Connectors <ExternalLink size={11} /></button>
          </div>
        </details>
      </section>
    </div>
  </div>
</section>
