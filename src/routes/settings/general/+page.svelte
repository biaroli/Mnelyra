<script lang="ts">
  import { onMount } from "svelte";
  import { message } from "@tauri-apps/plugin-dialog";
  import { RefreshCw, Wrench } from "@lucide/svelte";
  import RuntimePolicyForm, {
    type RuntimePolicyDraft,
  } from "$lib/components/RuntimePolicyForm.svelte";
  import ActionsPolicyForm, {
    type ActionsPolicyDraft,
  } from "$lib/components/ActionsPolicyForm.svelte";
  import SecretInput from "$lib/components/SecretInput.svelte";
  import {
    getGlobalGeneral,
    listFrpProfiles,
    setGlobalGeneral,
    type FrpProfileDto,
  } from "$lib/api/settings";
  import { getSharedSecret, setSharedSecret } from "$lib/api/secrets";
  import {
    installSoftware,
    listSoftware,
    type SoftwareStatus,
  } from "$lib/api/software";
  import type { GlobalGeneralConfig } from "$lib/types";

  let general = $state<GlobalGeneralConfig | null>(null);
  let frpProfiles = $state<FrpProfileDto[]>([]);
  let software = $state<SoftwareStatus[]>([]);
  let cloudflareToken = $state("");
  let actionsCloudflareToken = $state("");
  let originalCloudflareToken = $state("");
  let originalActionsCloudflareToken = $state("");
  let loading = $state(true);
  let saving = $state(false);
  let repairing = $state(false);

  const mcpTunnel = $derived(general?.mcpTunnel);
  const actions = $derived(general?.actions);

  async function refresh() {
    loading = true;
    try {
      const [nextGeneral, nextFrp, nextSoftware, mcpToken, actionsToken] =
        await Promise.all([
          getGlobalGeneral(),
          listFrpProfiles(),
          listSoftware(),
          getSharedSecret("cloudflare_token"),
          getSharedSecret("actions_cloudflare_token"),
        ]);
      general = nextGeneral;
      frpProfiles = nextFrp;
      software = nextSoftware;
      cloudflareToken = mcpToken ?? "";
      actionsCloudflareToken = actionsToken ?? "";
      originalCloudflareToken = cloudflareToken;
      originalActionsCloudflareToken = actionsCloudflareToken;
    } catch (error) {
      await message(String(error), { title: "加载通用设置失败", kind: "error" });
    } finally {
      loading = false;
    }
  }

  function updateMcpRuntime<K extends keyof GlobalGeneralConfig["mcpRuntime"]>(
    key: K,
    value: GlobalGeneralConfig["mcpRuntime"][K],
  ) {
    if (!general) return;
    general = {
      ...general,
      mcpRuntime: { ...general.mcpRuntime, [key]: value },
    };
  }

  function updateMcpTunnel<K extends keyof GlobalGeneralConfig["mcpTunnel"]>(
    key: K,
    value: GlobalGeneralConfig["mcpTunnel"][K],
  ) {
    if (!general) return;
    general = {
      ...general,
      mcpTunnel: { ...general.mcpTunnel, [key]: value },
    };
  }

  function updateActions<K extends keyof GlobalGeneralConfig["actions"]>(
    key: K,
    value: GlobalGeneralConfig["actions"][K],
  ) {
    if (!general) return;
    general = {
      ...general,
      actions: { ...general.actions, [key]: value },
    };
  }

  async function saveAll() {
    if (!general || saving) return;
    saving = true;
    try {
      if (cloudflareToken !== originalCloudflareToken) {
        if (!cloudflareToken.trim()) throw new Error("MCP Cloudflare Tunnel Token 不能为空");
        await setSharedSecret("cloudflare_token", cloudflareToken.trim());
        originalCloudflareToken = cloudflareToken.trim();
      }
      if (actionsCloudflareToken !== originalActionsCloudflareToken) {
        if (!actionsCloudflareToken.trim()) throw new Error("Actions Cloudflare Tunnel Token 不能为空");
        await setSharedSecret("actions_cloudflare_token", actionsCloudflareToken.trim());
        originalActionsCloudflareToken = actionsCloudflareToken.trim();
      }
      await setGlobalGeneral(general);
      await refresh();
    } catch (error) {
      await message(String(error), { title: "保存通用设置失败", kind: "error" });
    } finally {
      saving = false;
    }
  }

  async function saveMcpPolicy(draft: RuntimePolicyDraft) {
    if (!general) return;
    const next: GlobalGeneralConfig = {
      ...general,
      mcpRuntime: {
        ...general.mcpRuntime,
        tool_profile: draft.toolProfile,
        permission_mode: draft.permissionMode,
        allowed_commands: draft.allowedCommands,
        workspace_local_entries: draft.workspaceLocalEntries,
        workspace_script_extensions: draft.workspaceScriptExtensions,
      },
    };
    general = next;
    await setGlobalGeneral(next);
  }

  async function saveActionsPolicy(draft: ActionsPolicyDraft) {
    if (!general) return;
    const next: GlobalGeneralConfig = {
      ...general,
      actions: {
        ...general.actions,
        allowed_commands: draft.allowedCommands,
        max_patch_bytes: draft.maxPatchBytes,
        permission_mode: draft.permissionMode,
      },
    };
    general = next;
    await setGlobalGeneral(next);
  }

  async function repairComponents() {
    if (repairing) return;
    repairing = true;
    try {
      const results = await Promise.allSettled([
        installSoftware("frpc"),
        installSoftware("cloudflared"),
      ]);
      software = await listSoftware();
      const failed = results.filter((result) => result.status === "rejected");
      if (failed.length > 0) {
        const reasons = failed
          .map((result) => (result.status === "rejected" ? String(result.reason) : ""))
          .filter(Boolean)
          .join("\n");
        throw new Error(reasons || "部分隧道组件修复失败");
      }
      await message("frpc 与 cloudflared 已重新检查并安装。", {
        title: "组件修复完成",
        kind: "info",
      });
    } catch (error) {
      await message(String(error), { title: "组件修复失败", kind: "error" });
    } finally {
      repairing = false;
    }
  }

  onMount(() => {
    void refresh();
  });
</script>

<section class="page-scroll">
  <header class="page-header">
    <p class="page-kicker">全局设置</p>
    <h2 class="page-title">通用</h2>
    <p class="mt-2 max-w-3xl text-sm text-[var(--color-text-muted)]">
      RootRelay 全局只运行一套 MCP / Actions 配置。切换工作区只切换项目根目录，不再复制端口、隧道、策略或认证配置。
    </p>
  </header>

  <div class="page-body flex flex-col gap-6">
    {#if loading || !general}
      <div class="tx-card p-4 text-sm text-[var(--color-text-muted)]">加载中…</div>
    {:else}
      <div class="tx-card p-4">
        <div class="flex items-start justify-between gap-4">
          <div>
            <h3 class="text-sm font-semibold">隧道组件</h3>
            <p class="mt-1 text-xs text-[var(--color-text-muted)]">
              应用启动时会自动检查缺失组件。这里的“修复组件”会重新下载安装缓存副本，用于缺失或损坏恢复。
            </p>
          </div>
          <button
            type="button"
            class="tx-btn-ghost inline-flex shrink-0 items-center gap-2"
            disabled={repairing}
            onclick={() => void repairComponents()}
          >
            {#if repairing}<RefreshCw size={14} class="animate-spin" />{:else}<Wrench size={14} />{/if}
            {repairing ? "修复中…" : "修复组件"}
          </button>
        </div>
        <div class="mt-4 grid gap-2 md:grid-cols-2">
          {#each software as item (item.kind)}
            <div class="tx-panel px-3 py-2">
              <div class="flex items-center justify-between gap-3">
                <span class="text-sm font-medium">{item.name}</span>
                <span class="text-xs {item.installed ? 'text-[var(--color-accent)]' : 'text-[var(--danger)]'}">
                  {item.installed ? "已安装" : "缺失"}
                </span>
              </div>
              <p class="tx-mono mt-1 truncate text-xs text-[var(--color-text-muted)]">
                {item.path || "等待自动安装"}
              </p>
            </div>
          {/each}
        </div>
      </div>

      <div class="tx-card p-5">
        <p class="tx-section-label">MCP 服务</p>
        <div class="mt-4 grid gap-4 md:grid-cols-2">
          <label class="grid gap-1">
            <span class="text-xs text-[var(--color-text-muted)]">本地端口</span>
            <input
              type="number"
              min="1024"
              max="65535"
              class="tx-input"
              value={general.mcpRuntime.local_port}
              oninput={(event) => updateMcpRuntime("local_port", Number(event.currentTarget.value))}
            />
          </label>
          <label class="grid gap-1">
            <span class="text-xs text-[var(--color-text-muted)]">隧道类型</span>
            <select
              class="tx-input"
              value={general.mcpTunnel.type}
              onchange={(event) => updateMcpTunnel("type", event.currentTarget.value)}
            >
              <option value="none">不启用公网隧道</option>
              <option value="cloudflare">Cloudflare</option>
              <option value="frp">FRP</option>
            </select>
          </label>
        </div>

        {#if general.mcpTunnel.type === "cloudflare"}
          <div class="mt-4 grid gap-4 md:grid-cols-2">
            <label class="grid gap-1">
              <span class="text-xs text-[var(--color-text-muted)]">Cloudflare 模式</span>
              <select
                class="tx-input"
                value={general.mcpTunnel.cloudflare_mode}
                onchange={(event) => updateMcpTunnel("cloudflare_mode", event.currentTarget.value)}
              >
                <option value="quick">Quick Tunnel</option>
                <option value="named">Named Tunnel</option>
              </select>
            </label>
            <label class="grid gap-1">
              <span class="text-xs text-[var(--color-text-muted)]">固定公网 URL</span>
              <input
                class="tx-input tx-mono"
                type="text"
                placeholder="https://mcp.example.com"
                value={general.mcpTunnel.public_url}
                oninput={(event) => updateMcpTunnel("public_url", event.currentTarget.value)}
              />
            </label>
          </div>
          {#if general.mcpTunnel.cloudflare_mode === "named"}
            <div class="mt-4 grid gap-1">
              <span class="text-xs text-[var(--color-text-muted)]">Cloudflare Tunnel Token</span>
              <SecretInput bind:value={cloudflareToken} />
            </div>
          {/if}
        {:else if general.mcpTunnel.type === "frp"}
          <div class="mt-4 grid gap-4 md:grid-cols-2">
            <label class="grid gap-1">
              <span class="text-xs text-[var(--color-text-muted)]">FRP 配置</span>
              <select
                class="tx-input"
                value={general.mcpTunnel.frp_profile_id ?? ""}
                onchange={(event) => updateMcpTunnel("frp_profile_id", event.currentTarget.value)}
              >
                <option value="">手动服务器</option>
                {#each frpProfiles as item (item.id)}
                  <option value={item.id}>{item.name}</option>
                {/each}
              </select>
            </label>
            <label class="grid gap-1">
              <span class="text-xs text-[var(--color-text-muted)]">子域名</span>
              <input
                class="tx-input tx-mono"
                value={general.mcpTunnel.frp_subdomain}
                oninput={(event) => updateMcpTunnel("frp_subdomain", event.currentTarget.value)}
              />
            </label>
            {#if !(general.mcpTunnel.frp_profile_id ?? "")}
              <label class="grid gap-1">
                <span class="text-xs text-[var(--color-text-muted)]">FRP 服务器</span>
                <input
                  class="tx-input tx-mono"
                  value={general.mcpTunnel.frp_server}
                  oninput={(event) => updateMcpTunnel("frp_server", event.currentTarget.value)}
                />
              </label>
              <label class="grid gap-1">
                <span class="text-xs text-[var(--color-text-muted)]">FRP 端口</span>
                <input
                  type="number"
                  class="tx-input"
                  value={general.mcpTunnel.frp_server_port ?? 7000}
                  oninput={(event) => updateMcpTunnel("frp_server_port", Number(event.currentTarget.value))}
                />
              </label>
            {/if}
          </div>
        {/if}

        <div class="mt-6 border-t border-[var(--color-border)] pt-5">
          <p class="tx-section-label">MCP 策略</p>
          <div class="mt-3">
            <RuntimePolicyForm
              toolProfile={general.mcpRuntime.tool_profile}
              permissionMode={general.mcpRuntime.permission_mode}
              allowedCommands={general.mcpRuntime.allowed_commands ?? ""}
              workspaceLocalEntries={general.mcpRuntime.workspace_local_entries ?? true}
              workspaceScriptExtensions={general.mcpRuntime.workspace_script_extensions ?? ".exe,.bat,.cmd,.ps1"}
              onSave={saveMcpPolicy}
            />
          </div>
        </div>
      </div>

      <div class="tx-card p-5">
        <p class="tx-section-label">Actions 服务</p>
        <div class="mt-4 grid gap-4 md:grid-cols-2">
          <label class="grid gap-1">
            <span class="text-xs text-[var(--color-text-muted)]">本地端口</span>
            <input
              type="number"
              min="1024"
              max="65535"
              class="tx-input"
              value={general.actions.local_port}
              oninput={(event) => updateActions("local_port", Number(event.currentTarget.value))}
            />
          </label>
          <label class="grid gap-1">
            <span class="text-xs text-[var(--color-text-muted)]">隧道类型</span>
            <select
              class="tx-input"
              value={general.actions.tunnel_type}
              onchange={(event) => updateActions("tunnel_type", event.currentTarget.value)}
            >
              <option value="none">不启用公网隧道</option>
              <option value="cloudflare">Cloudflare</option>
              <option value="frp">FRP</option>
            </select>
          </label>
        </div>

        {#if general.actions.tunnel_type === "cloudflare"}
          <div class="mt-4 grid gap-4 md:grid-cols-2">
            <label class="grid gap-1">
              <span class="text-xs text-[var(--color-text-muted)]">Cloudflare 模式</span>
              <select
                class="tx-input"
                value={general.actions.cloudflare_mode}
                onchange={(event) => updateActions("cloudflare_mode", event.currentTarget.value)}
              >
                <option value="quick">Quick Tunnel</option>
                <option value="named">Named Tunnel</option>
              </select>
            </label>
            <label class="grid gap-1">
              <span class="text-xs text-[var(--color-text-muted)]">固定公网 URL</span>
              <input
                class="tx-input tx-mono"
                value={general.actions.public_url}
                oninput={(event) => updateActions("public_url", event.currentTarget.value)}
              />
            </label>
          </div>
          {#if general.actions.cloudflare_mode === "named"}
            <div class="mt-4 grid gap-1">
              <span class="text-xs text-[var(--color-text-muted)]">Actions Cloudflare Tunnel Token</span>
              <SecretInput bind:value={actionsCloudflareToken} />
            </div>
          {/if}
        {:else if general.actions.tunnel_type === "frp"}
          <div class="mt-4 grid gap-4 md:grid-cols-2">
            <label class="grid gap-1">
              <span class="text-xs text-[var(--color-text-muted)]">FRP 配置</span>
              <select
                class="tx-input"
                value={general.actions.frp_profile_id ?? ""}
                onchange={(event) => updateActions("frp_profile_id", event.currentTarget.value)}
              >
                <option value="">手动服务器</option>
                {#each frpProfiles as item (item.id)}
                  <option value={item.id}>{item.name}</option>
                {/each}
              </select>
            </label>
            <label class="grid gap-1">
              <span class="text-xs text-[var(--color-text-muted)]">子域名</span>
              <input
                class="tx-input tx-mono"
                value={general.actions.frp_subdomain}
                oninput={(event) => updateActions("frp_subdomain", event.currentTarget.value)}
              />
            </label>
          </div>
        {/if}

        <div class="mt-6 border-t border-[var(--color-border)] pt-5">
          <p class="tx-section-label">Actions 策略</p>
          <div class="mt-3">
            <ActionsPolicyForm
              allowedCommands={general.actions.allowed_commands ?? ""}
              maxPatchBytes={general.actions.max_patch_bytes ?? 200_000}
              permissionMode={general.actions.permission_mode}
              onSave={saveActionsPolicy}
            />
          </div>
        </div>
      </div>

      <div class="flex justify-end pb-2">
        <button
          type="button"
          class="tx-btn-primary px-4 py-2"
          disabled={saving}
          onclick={() => void saveAll()}
        >
          {saving ? "保存中…" : "保存设置"}
        </button>
      </div>
    {/if}
  </div>
</section>
