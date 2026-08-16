<script lang="ts">
  import { onMount } from "svelte";
  import { message } from "@tauri-apps/plugin-dialog";
  import CopyButton from "$lib/components/CopyButton.svelte";
  import SecretInput from "$lib/components/SecretInput.svelte";
  import ServicePanel from "$lib/components/ServicePanel.svelte";
  import { getRuntimeStatus } from "$lib/api/workspaces";
  import { getGlobalAuth, getGlobalGeneral, setGlobalAuth } from "$lib/api/settings";
  import {
    getSharedSecret,
    regenerateSharedSecret,
    type SharedSecretKey,
  } from "$lib/api/secrets";
  import { activeWorkspaceState } from "$lib/stores/app";
  import { uiLocale } from "$lib/stores/locale";
  import {
    mcpLocalEndpoint,
    type GlobalAuthConfig,
    type GlobalGeneralConfig,
    type RuntimeStatus,
  } from "$lib/types";

  type SecretRow = {
    key: SharedSecretKey;
    label: string;
    stableId?: boolean;
  };

  const MCP_ROWS: SecretRow[] = [
    {
      key: "oauth_client_id",
      label: "OAuth Client ID",
      stableId: true,
    },
    { key: "oauth_approval_code", label: "Authorization code" },
    { key: "bearer_token", label: "Bearer Token" },
  ];

  const ALL_ROWS = MCP_ROWS;

  let auth = $state<GlobalAuthConfig | null>(null);
  let general = $state<GlobalGeneralConfig | null>(null);
  let runtime = $state<RuntimeStatus | null>(null);
  let secrets = $state<Record<string, string>>({});
  let loading = $state(true);
  let saving = $state(false);
  let regenerating = $state<string | null>(null);
  const zh = $derived($uiLocale === "zh-CN");

  function rowLabel(row: SecretRow): string {
    if (!zh) return row.label;
    if (row.key === "oauth_approval_code") return "授权码";
    if (row.key === "bearer_token") return "连接密钥";
    return row.label;
  }

  async function refreshRuntime(workspaceId: string | null) {
    if (!workspaceId) {
      runtime = null;
      return;
    }
    try {
      runtime = await getRuntimeStatus(workspaceId);
    } catch {
      runtime = null;
    }
  }

  $effect(() => {
    void refreshRuntime($activeWorkspaceState.workspaceId);
  });

  async function refresh() {
    loading = true;
    try {
      const [nextAuth, nextGeneral, loaded] = await Promise.all([
        getGlobalAuth(),
        getGlobalGeneral(),
        Promise.all(
          ALL_ROWS.map(async ({ key }) => [key, (await getSharedSecret(key)) ?? ""] as const),
        ),
      ]);
      auth = nextAuth;
      general = nextGeneral;
      secrets = Object.fromEntries(loaded);
    } catch (error) {
      await message(String(error), { title: zh ? "加载认证设置失败" : "Failed to load authentication settings", kind: "error" });
    } finally {
      loading = false;
    }
  }

  async function regenerate(key: SharedSecretKey) {
    if (regenerating) return;
    regenerating = key;
    try {
      const value = await regenerateSharedSecret(key);
      secrets = { ...secrets, [key]: value };
    } catch (error) {
      await message(String(error), { title: zh ? "重新生成失败" : "Failed to regenerate secret", kind: "error" });
    } finally {
      regenerating = null;
    }
  }

  async function save() {
    if (!auth || saving) return;
    saving = true;
    try {
      await setGlobalAuth(auth);
      await refresh();
    } catch (error) {
      await message(String(error), { title: zh ? "保存认证设置失败" : "Failed to save authentication settings", kind: "error" });
    } finally {
      saving = false;
    }
  }

  function updateAuth<K extends keyof GlobalAuthConfig>(key: K, value: GlobalAuthConfig[K]) {
    if (!auth) return;
    auth = { ...auth, [key]: value };
  }

  onMount(() => {
    void refresh();
    const timer = window.setInterval(() => {
      void refreshRuntime($activeWorkspaceState.workspaceId);
    }, 3000);
    return () => window.clearInterval(timer);
  });
</script>

<section class="page-scroll">
  <header class="page-header">
    <p class="page-kicker">{zh ? "访问控制" : "ACCESS CONTROL"}</p>
    <h2 class="page-title">{zh ? "认证" : "Authentication"}</h2>
    <p class="mt-2 max-w-3xl text-sm text-[var(--color-text-muted)]">{zh ? "统一管理 MCP 连接地址、固定身份与授权密钥。" : "Manage the shared MCP endpoint, identity, and authorization credentials."}</p>
  </header>

  <div class="page-body flex flex-col gap-6">
    {#if loading || !auth}
      <div class="tx-card p-4 text-sm text-[var(--color-text-muted)]">{zh ? "加载中…" : "Loading…"}</div>
    {:else}
      {#if general}
        <ServicePanel
          title="MCP"
          subtitle={zh ? "所有工作区共用的连接入口；切换工作区不会改变客户端地址" : "Shared connection entry for every workspace; switching workspaces does not change the client endpoint"}
          status={runtime?.state ?? "stopped"}
          statusMessage={runtime?.localMessage ?? ""}
          port={general.mcpRuntime.local_port}
          portEditable={false}
          tunnelType={general.mcpTunnel.type}
          localEndpoint={runtime?.localEndpoint || mcpLocalEndpoint(general.mcpRuntime.local_port)}
          publicEndpoint={runtime?.publicEndpoint ?? ""}
          publicLabel={zh ? "公网 MCP" : "Public MCP"}
          showToggle={false}
        />
      {/if}

      <div class="tx-card p-5">
        <p class="tx-section-label">{zh ? "MCP 认证" : "MCP authentication"}</p>
        <label class="mt-4 grid gap-1">
          <span class="text-xs text-[var(--color-text-muted)]">{zh ? "认证类型" : "Authentication type"}</span>
          <select
            class="tx-input"
            value={auth.mcpAuthType}
            onchange={(event) => updateAuth("mcpAuthType", event.currentTarget.value)}
          >
            <option value="oauth">OAuth</option>
            <option value="bearer">Bearer Token</option>
          </select>
        </label>

        <div class="mt-5 grid gap-4">
          {#each MCP_ROWS as row (row.key)}
            {#if auth.mcpAuthType === "oauth" && (row.key === "oauth_client_id" || row.key === "oauth_approval_code") || auth.mcpAuthType === "bearer" && row.key === "bearer_token"}
              <div class="grid gap-1">
                <span class="text-xs text-[var(--color-text-muted)]">{rowLabel(row)}</span>
                {#if row.stableId}
                  <div class="mn-fixed-credential">
                    <code>{secrets[row.key] || "—"}</code>
                    {#if secrets[row.key]}<CopyButton value={secrets[row.key]} />{/if}
                  </div>
                {:else}
                  <SecretInput
                    value={secrets[row.key]}
                    readonly
                    onRegenerate={() => void regenerate(row.key)}
                    regenerating={regenerating === row.key}
                  />
                {/if}
              </div>
            {/if}
          {/each}
        </div>
      </div>

      <div class="flex justify-end pb-2">
        <button
          type="button"
          class="tx-btn-primary px-4 py-2"
          disabled={saving}
          onclick={() => void save()}
        >
          {saving ? (zh ? "保存中…" : "Saving…") : (zh ? "保存设置" : "Save settings")}
        </button>
      </div>
    {/if}
  </div>
</section>
