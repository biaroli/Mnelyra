<script lang="ts">
  import { onMount } from "svelte";
  import { Cloud, RadioTower, Save, Wrench } from "@lucide/svelte";
  import SecretInput from "$lib/components/SecretInput.svelte";
  import {
    getGlobalGeneral,
    setGlobalGeneral,
  } from "$lib/api/settings";
  import { getSharedSecret, setSharedSecret } from "$lib/api/secrets";
  import { installSoftware, listSoftware, type SoftwareStatus } from "$lib/api/software";
  import { getRuntimeStatus } from "$lib/api/workspaces";
  import { activeWorkspaceState } from "$lib/stores/app";
  import { showToast } from "$lib/stores/toast";
  import { uiLocale } from "$lib/stores/locale";
  import type { GlobalGeneralConfig } from "$lib/types";

  let general = $state<GlobalGeneralConfig | null>(null);
  let software = $state<SoftwareStatus[]>([]);
  let cloudflareToken = $state("");
  let frpToken = $state("");
  let originalCloudflareToken = $state("");
  let originalFrpToken = $state("");
  let busy = $state(false);
  let repairing = $state(false);
  let loading = $state(true);
  let quickPublicUrl = $state("");
  let quickEndpointLoading = $state(false);
  let quickCopied = $state(false);
  let quickCopyTimer: number | undefined;

  const zh = $derived($uiLocale === "zh-CN");

  async function refreshQuickEndpoint() {
    const workspaceId = $activeWorkspaceState.workspaceId;
    if (!workspaceId) {
      quickPublicUrl = "";
      quickEndpointLoading = false;
      return;
    }
    try {
      const runtime = await getRuntimeStatus(workspaceId);
      quickPublicUrl = runtime.publicEndpoint?.trim() ?? "";
      quickEndpointLoading = runtime.state === "running" && !quickPublicUrl;
    } catch {
      quickPublicUrl = "";
      quickEndpointLoading = false;
    }
  }

  async function copyQuickEndpoint() {
    if (!quickPublicUrl) return;
    await navigator.clipboard.writeText(quickPublicUrl);
    quickCopied = true;
    if (quickCopyTimer !== undefined) window.clearTimeout(quickCopyTimer);
    quickCopyTimer = window.setTimeout(() => {
      quickCopied = false;
      quickCopyTimer = undefined;
    }, 1400);
  }

  async function refresh() {
    loading = true;
    try {
      const [nextGeneral, nextSoftware, mcpToken, nextFrpToken] = await Promise.all([
        getGlobalGeneral(),
        listSoftware(),
        getSharedSecret("cloudflare_token"),
        getSharedSecret("frp_token"),
      ]);
      general = nextGeneral;
      software = nextSoftware;
      cloudflareToken = mcpToken ?? "";
      frpToken = nextFrpToken ?? "";
      originalCloudflareToken = cloudflareToken;
      originalFrpToken = frpToken;
      await refreshQuickEndpoint();
    } catch (error) {
      showToast(String(error), { title: zh ? "连接配置读取失败" : "Failed to load connection routing", kind: "error", duration: 9000 });
    } finally {
      loading = false;
    }
  }

  function updateMcpTunnel<K extends keyof GlobalGeneralConfig["mcpTunnel"]>(
    key: K,
    value: GlobalGeneralConfig["mcpTunnel"][K],
  ) {
    if (!general) return;
    general = { ...general, mcpTunnel: { ...general.mcpTunnel, [key]: value } };
  }

  async function saveRouting() {
    if (!general || busy) return;
    busy = true;
    try {
      if (cloudflareToken !== originalCloudflareToken) {
        if (!cloudflareToken.trim()) throw new Error(zh ? "MCP Cloudflare Token 不能为空" : "MCP Cloudflare token cannot be empty");
        await setSharedSecret("cloudflare_token", cloudflareToken.trim());
        originalCloudflareToken = cloudflareToken.trim();
      }
      if (frpToken !== originalFrpToken && frpToken.trim()) {
        await setSharedSecret("frp_token", frpToken.trim());
        originalFrpToken = frpToken.trim();
      }
      await setGlobalGeneral(general);
      await refresh();
      showToast(zh ? "连接路由已保存并重新验证。" : "Connection routing saved and revalidated.", { title: zh ? "路由已更新" : "Routing updated", kind: "success" });
    } catch (error) {
      showToast(String(error), { title: zh ? "连接路由保存失败" : "Failed to save routing", kind: "error", duration: 10000 });
    } finally {
      busy = false;
    }
  }

  async function repairNetworkComponents() {
    if (repairing) return;
    repairing = true;
    try {
      await Promise.allSettled([installSoftware("frpc"), installSoftware("cloudflared")]);
      software = await listSoftware();
      showToast(zh ? "连接组件已重新检查。" : "Connection components rechecked.", { title: zh ? "诊断完成" : "Diagnostics complete", kind: "success" });
    } catch (error) {
      showToast(String(error), { title: zh ? "组件修复失败" : "Component repair failed", kind: "error", duration: 9000 });
    } finally {
      repairing = false;
    }
  }

  onMount(() => {
    void refresh();
    const timer = window.setInterval(() => {
      if (general?.mcpTunnel.type === "cloudflare" && general.mcpTunnel.cloudflare_mode === "quick") {
        void refreshQuickEndpoint();
      }
    }, 2500);
    return () => {
      window.clearInterval(timer);
      if (quickCopyTimer !== undefined) window.clearTimeout(quickCopyTimer);
    };
  });
</script>

<section class="mn-console-panel mn-routing-panel">
  <div class="mn-panel-cap">
    <div>
      <span>{zh ? "公网路由" : "PUBLIC ROUTING"}</span>
      <strong>{zh ? "长期连接入口" : "Long-lived entry"}</strong>
    </div>
    <b>{general?.mcpTunnel.type === "none" ? (zh ? "未配置" : "NOT SET") : general?.mcpTunnel.type?.toUpperCase() ?? "—"}</b>
  </div>

  {#if loading || !general}
    <div class="mn-routing-loading">{zh ? "读取连接路由…" : "Reading connection routing…"}</div>
  {:else}
    <div class="mn-route-mode-grid" role="group" aria-label={zh ? "MCP 公网接入方式" : "MCP public routing mode"}>
      <button type="button" class:active={general.mcpTunnel.type === "cloudflare"} onclick={() => updateMcpTunnel("type", "cloudflare")}>
        <Cloud size={15} />
        <span>Cloudflare</span>
      </button>
      <button type="button" class:active={general.mcpTunnel.type === "frp"} onclick={() => updateMcpTunnel("type", "frp")}>
        <RadioTower size={15} />
        <span>FRP</span>
        <small>{zh ? "自托管反向代理" : "Self-hosted relay"}</small>
      </button>
    </div>

    {#if general.mcpTunnel.type === "cloudflare"}
      <div class="mn-route-editor">
        <label>
          <span>{zh ? "Cloudflare 模式" : "CLOUDFLARE MODE"}</span>
          <select value={general.mcpTunnel.cloudflare_mode} onchange={(event) => updateMcpTunnel("cloudflare_mode", event.currentTarget.value)}>
            <option value="named">{zh ? "域名" : "Domain"}</option>
            <option value="quick">Quick</option>
          </select>
        </label>

        {#if general.mcpTunnel.cloudflare_mode === "named"}
        <label>
          <span>{zh ? "公网地址" : "PUBLIC URL"}</span>
          <input class="mono" value={general.mcpTunnel.public_url} placeholder="https://mcp.example.com" oninput={(event) => updateMcpTunnel("public_url", event.currentTarget.value)} />
        </label>
        <label class="wide">
          <span>{zh ? "Tunnel Token" : "TUNNEL TOKEN"}</span>
          <SecretInput bind:value={cloudflareToken} />
        </label>
        {:else}
          <div class="mn-route-local-note mn-quick-endpoint wide">
            {#if quickPublicUrl}
              <span>{zh ? "临时地址" : "Temporary URL"}</span>
              <code>{quickPublicUrl}</code>
              <button type="button" class="mn-mini-action" onclick={() => void copyQuickEndpoint()}>
                {quickCopied ? (zh ? "已复制" : "Copied") : (zh ? "复制" : "Copy")}
              </button>
            {:else}
              <span>{quickEndpointLoading ? (zh ? "正在获取临时地址…" : "Getting temporary URL…") : (zh ? "尚未分配临时地址" : "No temporary URL assigned")}</span>
            {/if}
          </div>
        {/if}
      </div>
    {:else if general.mcpTunnel.type === "frp"}
      <div class="mn-route-editor mn-frp-active-editor">
        <label>
          <span>{zh ? "服务器" : "SERVER"}</span>
          <input class="mono" value={general.mcpTunnel.frp_server} placeholder="frp.example.com" oninput={(event) => updateMcpTunnel("frp_server", event.currentTarget.value)} />
        </label>
        <label>
          <span>{zh ? "端口" : "PORT"}</span>
          <input type="number" min="1" max="65535" value={general.mcpTunnel.frp_server_port ?? 7000} oninput={(event) => updateMcpTunnel("frp_server_port", Number(event.currentTarget.value))} />
        </label>
        <label>
          <span>{zh ? "子域名" : "SUBDOMAIN"}</span>
          <input class="mono" value={general.mcpTunnel.frp_subdomain} oninput={(event) => updateMcpTunnel("frp_subdomain", event.currentTarget.value)} />
        </label>
        <label class="wide">
          <span>Token</span>
          <SecretInput bind:value={frpToken} showCopy={false} />
        </label>
      </div>
    {/if}

    <div class="mn-routing-save-row">
      <span>{zh ? "保存后 Mnelyra 会事务式重建当前公网入口，不改变活动工作区。" : "Saving transactionally rebuilds the public entry without changing the active workspace."}</span>
      <button type="button" class="tx-btn-primary" disabled={busy} onclick={() => void saveRouting()}><Save size={13} /> {busy ? (zh ? "应用中…" : "Applying…") : (zh ? "应用路由" : "Apply routing")}</button>
    </div>

    <details class="mn-routing-advanced diagnostics">
      <summary>{zh ? "连接组件诊断" : "Connection component diagnostics"}</summary>
      <div class="mn-component-diagnostics">
        {#each software as item (item.kind)}
          <div class:ok={item.installed}><i></i><span>{item.name}</span><strong>{item.installed ? (zh ? "就绪" : "Ready") : (zh ? "缺失" : "Missing")}</strong></div>
        {/each}
        <button type="button" class="mn-mini-action" disabled={repairing} onclick={() => void repairNetworkComponents()}><Wrench size={12} /> {repairing ? (zh ? "检查中…" : "Checking…") : (zh ? "检查并修复" : "Check & repair")}</button>
      </div>
    </details>
  {/if}
</section>
