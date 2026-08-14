<script lang="ts">
  import { onMount } from "svelte";
  import { message } from "@tauri-apps/plugin-dialog";
  import SecretInput from "$lib/components/SecretInput.svelte";
  import { getGlobalAuth, setGlobalAuth } from "$lib/api/settings";
  import {
    getSharedSecret,
    regenerateSharedSecret,
    setSharedSecret,
    type SharedSecretKey,
  } from "$lib/api/secrets";
  import type { GlobalAuthConfig } from "$lib/types";

  type SecretRow = {
    key: SharedSecretKey;
    label: string;
    stableId?: boolean;
    hint?: string;
  };

  const MCP_ROWS: SecretRow[] = [
    {
      key: "oauth_client_id",
      label: "OAuth Client ID",
      stableId: true,
      hint: "RootRelay 首次安装时确定，此后整个安装生命周期固定。",
    },
    { key: "oauth_client_secret", label: "OAuth Client Secret" },
    { key: "oauth_password", label: "授权口令", hint: "客户端首次 OAuth 授权时输入" },
    { key: "oauth_token_secret", label: "OAuth Token Secret" },
    { key: "bearer_token", label: "Bearer Token" },
  ];

  const ACTIONS_ROWS: SecretRow[] = [
    {
      key: "actions_oauth_client_id",
      label: "OAuth Client ID",
      stableId: true,
      hint: "Actions 的固定客户端身份，不随重启或工作区切换变化。",
    },
    { key: "actions_oauth_client_secret", label: "OAuth Client Secret" },
    { key: "actions_oauth_password", label: "授权口令" },
    { key: "actions_oauth_token_secret", label: "OAuth Token Secret" },
    { key: "actions_api_key", label: "API Key（Bearer）" },
  ];

  const ALL_ROWS = [...MCP_ROWS, ...ACTIONS_ROWS];

  let auth = $state<GlobalAuthConfig | null>(null);
  let secrets = $state<Record<string, string>>({});
  let originals = $state<Record<string, string>>({});
  let loading = $state(true);
  let saving = $state(false);
  let regenerating = $state<string | null>(null);

  const secretDirty = $derived(
    ALL_ROWS.some(
      ({ key, stableId }) => !stableId && secrets[key] !== undefined && secrets[key] !== originals[key],
    ),
  );

  async function refresh() {
    loading = true;
    try {
      const [nextAuth, loaded] = await Promise.all([
        getGlobalAuth(),
        Promise.all(
          ALL_ROWS.map(async ({ key }) => [key, (await getSharedSecret(key)) ?? ""] as const),
        ),
      ]);
      auth = nextAuth;
      secrets = Object.fromEntries(loaded);
      originals = Object.fromEntries(loaded);
    } catch (error) {
      await message(String(error), { title: "加载认证设置失败", kind: "error" });
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
      originals = { ...originals, [key]: value };
    } catch (error) {
      await message(String(error), { title: "重新生成失败", kind: "error" });
    } finally {
      regenerating = null;
    }
  }

  async function save() {
    if (!auth || saving) return;
    saving = true;
    try {
      for (const { key, stableId } of ALL_ROWS) {
        if (stableId) continue;
        const value = secrets[key] ?? "";
        if (value !== originals[key]) {
          if (!value.trim()) throw new Error(`${key} 不能为空`);
          await setSharedSecret(key, value.trim());
          originals = { ...originals, [key]: value.trim() };
        }
      }
      await setGlobalAuth(auth);
      await refresh();
    } catch (error) {
      await message(String(error), { title: "保存认证设置失败", kind: "error" });
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
  });
</script>

<section class="page-scroll">
  <header class="page-header">
    <p class="page-kicker">全局设置</p>
    <h2 class="page-title">认证</h2>
    <p class="mt-2 max-w-3xl text-sm text-[var(--color-text-muted)]">
      MCP 与 Actions 认证均为应用级配置。工作区切换只改变代码根目录，不改变连接器身份、授权口令或 Token。
    </p>
  </header>

  <div class="page-body flex flex-col gap-6">
    {#if loading || !auth}
      <div class="tx-card p-4 text-sm text-[var(--color-text-muted)]">加载中…</div>
    {:else}
      <div class="tx-card p-5">
        <p class="tx-section-label">MCP 认证</p>
        <label class="mt-4 grid gap-1">
          <span class="text-xs text-[var(--color-text-muted)]">认证类型</span>
          <select
            class="tx-input"
            value={auth.mcpAuthType}
            onchange={(event) => updateAuth("mcpAuthType", event.currentTarget.value)}
          >
            <option value="oauth">OAuth</option>
            <option value="bearer">Bearer Token</option>
            <option value="noauth">不启用认证</option>
          </select>
        </label>

        <div class="mt-5 grid gap-4">
          {#each MCP_ROWS as row (row.key)}
            {#if row.stableId || auth.mcpAuthType === "oauth" && row.key.startsWith("oauth_") || auth.mcpAuthType === "bearer" && row.key === "bearer_token"}
              <div class="grid gap-1">
                <div class="flex items-center justify-between gap-3">
                  <span class="text-xs text-[var(--color-text-muted)]">{row.label}</span>
                  {#if row.stableId}
                    <span class="text-[11px] text-[var(--color-text-muted)]">固定身份 不可轮换</span>
                  {/if}
                </div>
                <SecretInput
                  bind:value={secrets[row.key]}
                  readonly={row.stableId ?? false}
                  onRegenerate={row.stableId ? undefined : () => void regenerate(row.key)}
                  regenerating={regenerating === row.key}
                />
                {#if row.hint}
                  <p class="text-xs text-[var(--color-text-muted)]">{row.hint}</p>
                {/if}
              </div>
            {/if}
          {/each}
        </div>
      </div>

      <div class="tx-card p-5">
        <p class="tx-section-label">Actions 认证</p>
        <div class="mt-4 grid gap-4 md:grid-cols-2">
          <label class="grid gap-1">
            <span class="text-xs text-[var(--color-text-muted)]">认证类型</span>
            <select
              class="tx-input"
              value={auth.actionsAuthType}
              onchange={(event) => updateAuth("actionsAuthType", event.currentTarget.value)}
            >
              <option value="api_key">API Key</option>
              <option value="oauth">OAuth</option>
              <option value="none">不启用认证</option>
            </select>
          </label>
          {#if auth.actionsAuthType === "oauth"}
            <label class="grid gap-1">
              <span class="text-xs text-[var(--color-text-muted)]">OAuth Scopes</span>
              <input
                class="tx-input tx-mono"
                value={auth.actionsOauthScopes}
                oninput={(event) => updateAuth("actionsOauthScopes", event.currentTarget.value)}
                placeholder="scope1 scope2"
              />
            </label>
          {/if}
        </div>

        <div class="mt-5 grid gap-4">
          {#each ACTIONS_ROWS as row (row.key)}
            {#if row.stableId || auth.actionsAuthType === "oauth" && row.key.startsWith("actions_oauth_") || auth.actionsAuthType === "api_key" && row.key === "actions_api_key"}
              <div class="grid gap-1">
                <div class="flex items-center justify-between gap-3">
                  <span class="text-xs text-[var(--color-text-muted)]">{row.label}</span>
                  {#if row.stableId}
                    <span class="text-[11px] text-[var(--color-text-muted)]">固定身份 不可轮换</span>
                  {/if}
                </div>
                <SecretInput
                  bind:value={secrets[row.key]}
                  readonly={row.stableId ?? false}
                  onRegenerate={row.stableId ? undefined : () => void regenerate(row.key)}
                  regenerating={regenerating === row.key}
                />
                {#if row.hint}
                  <p class="text-xs text-[var(--color-text-muted)]">{row.hint}</p>
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
          disabled={saving || (!secretDirty && loading)}
          onclick={() => void save()}
        >
          {saving ? "保存中…" : "保存设置"}
        </button>
      </div>
    {/if}
  </div>
</section>
