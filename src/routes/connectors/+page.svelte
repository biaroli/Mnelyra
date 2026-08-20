<script lang="ts">
  import { onMount } from "svelte";
  import {
    getWebModelBridgeStatus,
    startWebModelBridge,
    stopWebModelBridge,
  } from "$lib/api/connectors";
  import { showToast } from "$lib/stores/toast";
  import { uiLocale } from "$lib/stores/locale";
  import { activeWorkspaceState, mcpRuntimeStates } from "$lib/stores/app";
  import { getRuntimeStatus } from "$lib/api/workspaces";
  import ConnectionRoutingPanel from "$lib/components/ConnectionRoutingPanel.svelte";
  import type { WebModelBridgeStatus } from "$lib/types";
  import { Play, Unplug } from "@lucide/svelte";

  let webModels = $state<WebModelBridgeStatus | null>(null);
  let publicRouteReady = $state(false);
  let busy = $state(false);
  let timer: number | undefined;
  const zh = $derived($uiLocale === "zh-CN");
  const mcpReady = $derived(
    $activeWorkspaceState.workspaceId != null
      && $mcpRuntimeStates[$activeWorkspaceState.workspaceId] === "running",
  );
  const connectionFlowing = $derived(mcpReady && publicRouteReady);
  const webModelTunnelRequired = $derived(webModels?.mode === "full");

  async function refresh() {
    try {
      const workspaceId = $activeWorkspaceState.workspaceId;
      const [nextWebModels, runtime] = await Promise.all([
        getWebModelBridgeStatus().catch(() => null),
        workspaceId ? getRuntimeStatus(workspaceId).catch(() => null) : Promise.resolve(null),
      ]);
      webModels = nextWebModels;
      publicRouteReady = Boolean(runtime?.state === "running" && runtime.publicEndpoint?.trim());
    } catch (error) {
      showToast(String(error), { title: zh ? "连接状态读取失败" : "Failed to read connector status", kind: "error", duration: 8000 });
    }
  }

  async function disconnectWebModels() {
    if (busy || webModels?.browserBusy) return;
    busy = true;
    try {
      webModels = await stopWebModelBridge();
      showToast(zh ? "网页模型已断开，Codex 原路由已恢复。" : "Web Models disconnected and the previous Codex route was restored.", {
        title: zh ? "已断开" : "Disconnected",
        kind: "success",
      });
    } catch (error) {
      showToast(String(error), { title: zh ? "断开失败" : "Disconnect failed", kind: "error", duration: 9000 });
    } finally {
      busy = false;
    }
  }

  async function recoverWebModels() {
    if (busy) return;
    busy = true;
    const installingRoute = !webModels?.routeInstalled;
    try {
      webModels = await startWebModelBridge();
      showToast(installingRoute
        ? (zh ? "网页模型路由已安装。新建一个 Codex 对话即可使用。" : "Web-model routing is installed. Start a new Codex conversation to use it.")
        : (zh ? "网页模型接入已恢复。" : "Web model access is ready."), {
        title: installingRoute ? (zh ? "已安装到 Codex" : "Installed in Codex") : (zh ? "网页模型已就绪" : "Web models ready"),
        kind: "success",
      });
    } catch (error) {
      showToast(String(error), { title: zh ? "网页模型恢复失败" : "Failed to recover web models", kind: "error", duration: 11000 });
      webModels = await getWebModelBridgeStatus().catch(() => webModels);
    } finally {
      busy = false;
    }
  }

  onMount(() => {
    void refresh();
    timer = window.setInterval(() => {
      void getWebModelBridgeStatus().then((value) => (webModels = value)).catch(() => {});
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
    // The Web-model status check includes the local browser-session probe. Five seconds keeps the
    // UI fresh without polling the ChatGPT session several times per second while this page is open.
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

      <section class="mn-connection-telemetry-panel mn-codex-bridge-panel" class:online={webModels?.ready}>
        <div class="mn-panel-cap">
          <div><span>MNELYRA</span><strong>{zh ? "网页模型接入" : "Web model bridge"}</strong></div>
          <b class:online={webModels?.ready}>
            {webModels?.ready ? (webModels.browserBusy ? (zh ? "任务中" : "BUSY") : (zh ? "已接入" : "READY")) : (webModels?.routeInstalled ? (zh ? "待恢复" : "RECOVER") : (zh ? "未接入" : "NOT CONNECTED"))}
          </b>
        </div>

        <div class="mn-codex-route-line" aria-label={zh ? "网页模型链路" : "Web model path"}>
          <span>CODEX</span><i>→</i><span>127.0.0.1:17841</span><i>→</i><span>CHATGPT WEB</span>
        </div>

        <div class="mn-codex-health-grid">
          <div class:ok={Boolean(webModels?.routeInstalled && webModels?.routeActive)}>
            <i></i><span>{zh ? "Codex 路由" : "Codex route"}</span><b>{webModels?.routeInstalled && webModels?.routeActive ? (zh ? "已接入" : "READY") : (zh ? "未接入" : "DOWN")}</b>
          </div>
          <div class:ok={Boolean(webModels?.proxyReady)}>
            <i></i><span>Responses</span><b>{webModels?.proxyReady ? "17841 OK" : "17841 DOWN"}</b>
          </div>
          <div class:ok={Boolean(webModels?.browserReady)}>
            <i></i><span>{zh ? "ChatGPT 浏览器" : "ChatGPT browser"}</span><b>{webModels?.browserBusy ? (zh ? "任务中" : "BUSY") : webModels?.browserReady ? (zh ? "已就绪" : "READY") : (zh ? "未就绪" : "DOWN")}</b>
          </div>
          <div class:ok={Boolean(!webModelTunnelRequired || webModels?.tunnelReady)}>
            <i></i><span>OpenAI Tunnel</span><b>{!webModelTunnelRequired ? (zh ? "无需" : "N/A") : webModels?.tunnelReady ? (zh ? "已就绪" : "READY") : (zh ? "未就绪" : "DOWN")}</b>
          </div>
        </div>

        <div class="mn-tunnel-live-row mn-codex-status-row">
          <div class="mn-tunnel-live-copy">
            <i class:ok={webModels?.ready}></i>
            <div>
              <strong>{zh ? "网页模型" : "Web models"}</strong>
              <span>
                {#if webModels?.ready}
                  {#if webModels.browserBusy}
                    {zh ? "正在运行 Codex 任务" : "Serving a Codex turn"}
                  {:else}
                    {zh ? "已接入 Codex" : "Connected to Codex"}
                  {/if}
                {:else if webModels?.routeInstalled && webModels?.routeActive}
                  {zh ? "Codex 路由已安装，但网页模型运行时未就绪。" : "The Codex route is installed, but the Web-model runtime is not ready."}
                {:else if webModels?.browserReady}
                  {zh ? "ChatGPT 会话已就绪，Codex 路由尚未安装。" : "ChatGPT session ready; Codex routing is not installed yet."}
                {:else if webModels?.browserRunning}
                  {zh ? "请在 Mnelyra 的 ChatGPT 窗口完成登录。" : "Finish signing in from Mnelyra's ChatGPT window."}
                {:else}
                  {zh ? "网页模型尚未启动。" : "Web Models have not been started."}
                {/if}
              </span>
              {#if webModels?.mode}
                <small>{webModels.mode.toUpperCase()}</small>
              {/if}
            </div>
          </div>
          {#if webModels?.ready}
            <button class="mn-mini-action" type="button" disabled={busy || webModels.browserBusy} onclick={() => void disconnectWebModels()}><Unplug size={12} /> {zh ? "断开" : "Disconnect"}</button>
          {:else if webModels?.codexDetected}
            <button class="mn-mini-action primary" type="button" disabled={busy} onclick={() => void recoverWebModels()}><Play size={12} /> {webModels?.routeInstalled ? (zh ? "恢复" : "Recover") : (zh ? "安装到 Codex" : "Install in Codex")}</button>
          {/if}
        </div>

      </section>
    </div>
  </div>
</section>
