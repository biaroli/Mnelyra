<script lang="ts">
  import { onMount } from "svelte";
  import { openUrl } from "$lib/api/app-info";
  import {
    getOpenAiConnectorSettings,
    getOpenAiConnectorStatus,
    saveOpenAiConnectorSettings,
    startOpenAiConnector,
    stopOpenAiConnector,
  } from "$lib/api/connectors";
  import { showToast } from "$lib/stores/toast";
  import { uiLocale } from "$lib/stores/locale";
  import { activeWorkspaceState, mcpRuntimeStates } from "$lib/stores/app";
  import { getRuntimeStatus } from "$lib/api/workspaces";
  import ConnectionRoutingPanel from "$lib/components/ConnectionRoutingPanel.svelte";
  import type { OpenAiConnectorSettings, OpenAiConnectorStatus } from "$lib/types";
  import { CircleStop, ExternalLink, Play, Save, ShieldCheck } from "@lucide/svelte";

  let settings = $state<OpenAiConnectorSettings | null>(null);
  let status = $state<OpenAiConnectorStatus | null>(null);
  let tunnelId = $state("");
  let alias = $state("mnelyra");
  let runtimeApiKey = $state("");
  let publicRouteReady = $state(false);
  let busy = $state(false);
  let timer: number | undefined;
  const zh = $derived($uiLocale === "zh-CN");
  const mcpReady = $derived(
    $activeWorkspaceState.workspaceId != null
      && $mcpRuntimeStates[$activeWorkspaceState.workspaceId] === "running",
  );
  const connectionFlowing = $derived(mcpReady && (publicRouteReady || Boolean(status?.ready)));

  const statusLabel = $derived(
    status?.ready
      ? (zh ? "已就绪" : "READY")
      : status?.processRunning
        ? status?.healthy
          ? (zh ? "连接中" : "STARTING")
          : (zh ? "异常" : "DEGRADED")
        : status?.configured
          ? (zh ? "未启动" : "STOPPED")
          : (zh ? "未配置" : "NOT CONFIGURED"),
  );

  async function refresh() {
    try {
      const workspaceId = $activeWorkspaceState.workspaceId;
      const [nextSettings, nextStatus, runtime] = await Promise.all([
        getOpenAiConnectorSettings(),
        getOpenAiConnectorStatus(),
        workspaceId ? getRuntimeStatus(workspaceId).catch(() => null) : Promise.resolve(null),
      ]);
      settings = nextSettings;
      status = nextStatus;
      publicRouteReady = Boolean(runtime?.state === "running" && runtime.publicEndpoint?.trim());
      if (!tunnelId) tunnelId = nextSettings.tunnelId;
      if (alias === "mnelyra" && nextSettings.alias) alias = nextSettings.alias;
    } catch (error) {
      showToast(String(error), { title: zh ? "连接状态读取失败" : "Failed to read connector status", kind: "error", duration: 8000 });
    }
  }

  async function persistSettings() {
    settings = await saveOpenAiConnectorSettings({
      tunnelId,
      alias,
      runtimeApiKey: runtimeApiKey.trim() || null,
    });
    runtimeApiKey = "";
    status = await getOpenAiConnectorStatus();
  }

  async function save() {
    if (busy) return;
    busy = true;
    try {
      await persistSettings();
      showToast(zh ? "OpenAI Tunnel 配置已保存。Runtime key 不会写入命令行。" : "OpenAI Tunnel settings saved. The runtime key is never written to the command line.", {
        title: zh ? "连接配置已保存" : "Connection saved",
        kind: "success",
      });
    } catch (error) {
      showToast(String(error), { title: zh ? "连接配置保存失败" : "Failed to save connection", kind: "error", duration: 9000 });
    } finally {
      busy = false;
    }
  }

  async function connect() {
    if (busy) return;
    busy = true;
    try {
      await persistSettings();
      status = await startOpenAiConnector();
      showToast(zh ? "OpenAI Tunnel 已通过进程、健康与就绪三重验证。" : "OpenAI Tunnel passed process, health, and readiness verification.", {
        title: zh ? "安全隧道已在线" : "Secure tunnel online",
        kind: "success",
      });
    } catch (error) {
      showToast(String(error), { title: zh ? "OpenAI Tunnel 连接失败" : "OpenAI Tunnel connection failed", kind: "error", duration: 11000 });
    } finally {
      busy = false;
    }
  }

  async function stop() {
    if (busy) return;
    busy = true;
    try {
      status = await stopOpenAiConnector();
    } catch (error) {
      showToast(String(error), { title: zh ? "Tunnel 停止失败" : "Failed to stop tunnel", kind: "error", duration: 9000 });
    } finally {
      busy = false;
    }
  }

  onMount(() => {
    void refresh();
    timer = window.setInterval(() => {
      void getOpenAiConnectorStatus().then((value) => (status = value)).catch(() => {});
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
    }, 2500);
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
      <div class="mn-map-node client"><span>{zh ? "上游" : "UPSTREAM"}</span><strong>{zh ? "MCP 客户端" : "MCP clients"}</strong></div>
      <div class="mn-map-wire"><i></i><b></b></div>
      <div class="mn-map-node relay"><span>{zh ? "连接" : "CONNECTION"}</span><strong>{zh ? "选择接入方式" : "Choose a connection path"}</strong></div>
      <div class="mn-map-wire"><i></i><b></b></div>
      <div class="mn-map-node core"><span>MNELYRA</span><strong>{zh ? "MCP 控制面" : "MCP control plane"}</strong></div>
      <div class="mn-map-wire"><i></i><b></b></div>
      <div class="mn-map-node workspace terminal"><strong>{zh ? "这台电脑" : "This computer"}</strong></div>
    </div>

    <div class="mn-connection-workbench">
      <ConnectionRoutingPanel />

      <section class="mn-connection-telemetry-panel" class:online={status?.ready}>
      <div class="mn-panel-cap">
        <div><span>OPENAI</span><strong>{zh ? "安全连接" : "Secure connection"}</strong></div>
        <b class:online={status?.ready}>{statusLabel}</b>
      </div>
      {#if status?.processRunning || status?.ready}
        <div class="mn-tunnel-live-row">
          <div class="mn-tunnel-live-copy">
            <i class:ok={status?.healthy}></i>
            <div>
              <strong>{status?.ready ? (zh ? "安全连接已可用" : "Secure connection is ready") : (zh ? "正在建立安全连接" : "Establishing secure connection")}</strong>
              <span>{zh ? "ChatGPT 可通过 Mnelyra 访问当前工作区。" : "ChatGPT can reach the active workspace through Mnelyra."}</span>
            </div>
          </div>
          <button class="mn-mini-action danger" type="button" disabled={busy} onclick={() => void stop()}><CircleStop size={12} /> {zh ? "断开" : "Stop"}</button>
        </div>
      {:else}
        <div class="mn-tunnel-empty">
          <div class="mn-tunnel-empty-copy">
            <ShieldCheck size={18} />
            <div>
              <strong>{status?.configured ? (zh ? "安全连接未启动" : "Secure connection is stopped") : (zh ? "安全连接尚未配置" : "Secure connection is not configured")}</strong>
              <span>{zh ? "这条连接只服务 OpenAI 平台；左侧公网路由不受影响。" : "This path is OpenAI-specific. Public routing on the left remains independent."}</span>
            </div>
          </div>
          <div class="mn-tunnel-actions">
            <button class="mn-mini-action primary" type="button" disabled={busy} onclick={() => void connect()}><Play size={12} /> {zh ? "启动" : "Start"}</button>
          </div>
        </div>
      {/if}
      <details class="mn-advanced-connection">
        <summary>{zh ? "高级配置" : "Advanced setup"}</summary>
        <div class="mn-connector-form">
          <label>
            <span>{zh ? "托管别名" : "MANAGED ALIAS"}</span>
            <input bind:value={alias} placeholder="mnelyra" disabled={busy} />
          </label>
          <label>
            <span>{zh ? "Tunnel ID" : "TUNNEL ID"}</span>
            <input bind:value={tunnelId} placeholder="tunnel_…" disabled={busy} autocomplete="off" />
          </label>
          <label class="wide">
            <span>{zh ? "OpenAI API Key" : "OPENAI API KEY"}</span>
            <input type="password" bind:value={runtimeApiKey} placeholder={settings?.hasRuntimeKey ? (zh ? "已保存 · 留空保持不变" : "saved · leave blank to keep") : (zh ? "粘贴 OpenAI API Key" : "paste OpenAI API key")} autocomplete="off" disabled={busy} />
          </label>
        </div>
        <div class="mn-connector-actions">
          <button class="mn-mini-action" type="button" disabled={busy} onclick={() => void save()}><Save size={12} /> {zh ? "保存配置" : "Save settings"}</button>
        </div>
      </details>

      <div class="mn-connector-links">
        <button type="button" onclick={() => void openUrl("https://platform.openai.com/settings/organization/tunnels")}>{zh ? "OpenAI Tunnels" : "OpenAI Tunnels"} <ExternalLink size={11} /></button>
        <button type="button" onclick={() => void openUrl("https://platform.openai.com/settings/organization/api-keys")}>OpenAI API Keys <ExternalLink size={11} /></button>
        <button type="button" onclick={() => void openUrl("https://chatgpt.com/#settings/Connectors")}>ChatGPT Connectors <ExternalLink size={11} /></button>
      </div>
      </section>

    </div>
  </div>
</section>
