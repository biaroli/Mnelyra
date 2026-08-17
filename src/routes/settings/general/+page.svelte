<script lang="ts">
  import { onMount } from "svelte";
  import { message } from "@tauri-apps/plugin-dialog";
  import { Globe2, Save, TerminalSquare } from "@lucide/svelte";
  import HealthPanel from "$lib/components/HealthPanel.svelte";
  import LogViewer from "$lib/components/LogViewer.svelte";
  import Tabs from "$lib/components/Tabs.svelte";
  import {
    getCodexContextPolicy,
    setCodexContextPolicy,
    setPermissionCeiling,
  } from "$lib/api/providers";
  import { getGlobalGeneral, setGlobalGeneral } from "$lib/api/settings";
  import { activeWorkspaceState, workspaces } from "$lib/stores/app";
  import { setUiLocale, uiLocale, type UiLocale } from "$lib/stores/locale";
  import { developerMode, setDeveloperMode } from "$lib/stores/developer";
  import { showToast } from "$lib/stores/toast";
  import { workspaceRootName, type CodexConfigReadResponse, type GlobalGeneralConfig } from "$lib/types";

  type DiagnosticsTab = "logs" | "health";

  let general = $state<GlobalGeneralConfig | null>(null);
  let loading = $state(true);
  let saving = $state(false);
  let codexPolicy = $state<CodexConfigReadResponse | null>(null);
  let compactPolicyLoading = $state(false);
  let compactBusy = $state(false);
  let permissionBusy = $state(false);
  let diagnosticsTab = $state<DiagnosticsTab>("logs");
  let contextMode = $state<"auto" | "one_million" | "custom">("auto");
  let contextWindowInput = $state("");
  let compactLimitInput = $state("");
  const zh = $derived($uiLocale === "zh-CN");
  const activeProfile = $derived(
    $workspaces.find((workspace) => workspace.id === $activeWorkspaceState.workspaceId) ?? null,
  );
  const diagnosticsTabs = $derived([
    { value: "logs", label: zh ? "日志" : "Logs" },
    { value: "health", label: zh ? "健康" : "Health" },
  ]);

  function compactLimitFrom(policy: CodexConfigReadResponse | null): number | null {
    const value = policy?.config?.model_auto_compact_token_limit;
    return typeof value === "number" && Number.isFinite(value) ? value : null;
  }

  function contextWindowFrom(policy: CodexConfigReadResponse | null): number | null {
    const value = policy?.config?.model_context_window;
    return typeof value === "number" && Number.isFinite(value) ? value : null;
  }

  const compactLimit = $derived(compactLimitFrom(codexPolicy));
  const contextWindow = $derived(contextWindowFrom(codexPolicy));

  async function refresh() {
    loading = true;
    try {
      general = await getGlobalGeneral();
    } catch (error) {
      await message(String(error), { title: zh ? "加载通用设置失败" : "Failed to load general settings", kind: "error" });
    } finally {
      loading = false;
    }
  }

  function updateMcpRuntime<K extends keyof GlobalGeneralConfig["mcpRuntime"]>(
    key: K,
    value: GlobalGeneralConfig["mcpRuntime"][K],
  ) {
    if (!general) return;
    general = { ...general, mcpRuntime: { ...general.mcpRuntime, [key]: value } };
  }

  async function savePorts() {
    if (!general || saving) return;
    saving = true;
    try {
      await setGlobalGeneral(general);
      await refresh();
    } catch (error) {
      await message(String(error), { title: zh ? "保存设置失败" : "Failed to save settings", kind: "error" });
    } finally {
      saving = false;
    }
  }

  function changeLocale(locale: UiLocale) {
    setUiLocale(locale);
  }

  async function loadCompactPolicy(force = false) {
    if (compactPolicyLoading || (!force && codexPolicy)) return;
    compactPolicyLoading = true;
    try {
      const next = await getCodexContextPolicy();
      const context = contextWindowFrom(next);
      const limit = compactLimitFrom(next);
      codexPolicy = next;
      contextMode = context == null && limit == null
        ? "auto"
        : context === 1_000_000 && limit === 900_000
          ? "one_million"
          : "custom";
      contextWindowInput = context?.toString() ?? "";
      compactLimitInput = limit?.toString() ?? "";
    } catch (error) {
      codexPolicy = null;
      showToast(String(error), {
        title: zh ? "读取总结策略失败" : "Failed to read compaction policy",
        kind: "error",
        duration: 9000,
      });
    } finally {
      compactPolicyLoading = false;
    }
  }

  async function changeDeveloperMode(enabled: boolean) {
    setDeveloperMode(enabled);
    if (enabled) await loadCompactPolicy();
  }

  async function saveContextPolicy(context: number | null, limit: number | null) {
    if (compactBusy) return;
    compactBusy = true;
    try {
      await setCodexContextPolicy(context, limit);
      await loadCompactPolicy(true);
      showToast(zh ? "Codex 上下文配置已保存。" : "Codex context settings saved.", {
        title: zh ? "已保存" : "Saved",
        kind: "success",
      });
    } catch (error) {
      showToast(String(error), {
        title: zh ? "上下文配置保存失败" : "Failed to save context settings",
        kind: "error",
        duration: 9000,
      });
    } finally {
      compactBusy = false;
    }
  }

  async function chooseContextMode(mode: "auto" | "one_million" | "custom") {
    if (mode === "custom") {
      contextMode = "custom";
      if (!contextWindowInput) contextWindowInput = contextWindow?.toString() || "1000000";
      if (!compactLimitInput) compactLimitInput = compactLimit?.toString() || "900000";
      return;
    }
    contextMode = mode;
    if (mode === "auto") {
      if (contextWindow != null || compactLimit != null) await saveContextPolicy(null, null);
      return;
    }
    await saveContextPolicy(1_000_000, 900_000);
  }

  async function saveCustomContextPolicy() {
    const context = Number(contextWindowInput);
    const compact = Number(compactLimitInput);
    if (!Number.isInteger(context) || context < 16_384) {
      showToast(
        zh ? "上下文窗口至少为 16,384。" : "Context window must be at least 16,384.",
        { title: zh ? "无效配置" : "Invalid settings", kind: "warning" },
      );
      return;
    }
    if (!Number.isInteger(compact) || compact < 16_384 || compact >= context) {
      showToast(
        zh ? "自动总结阈值至少为 16,384，并且必须小于上下文窗口。" : "Auto-compaction must be at least 16,384 and lower than the context window.",
        { title: zh ? "无效配置" : "Invalid settings", kind: "warning" },
      );
      return;
    }
    await saveContextPolicy(context, compact);
  }

  async function changePermissionCeiling(
    mode: GlobalGeneralConfig["permissionCeiling"],
  ) {
    if (!general || permissionBusy || general.permissionCeiling === mode) return;
    permissionBusy = true;
    const previous = general.permissionCeiling;
    general = { ...general, permissionCeiling: mode };
    try {
      await setPermissionCeiling(mode);
      await refresh();
      showToast(zh ? "权限总阀门已更新。" : "Permission ceiling updated.", {
        title: zh ? "已应用" : "Applied",
        kind: "success",
      });
    } catch (error) {
      general = general ? { ...general, permissionCeiling: previous } : general;
      showToast(String(error), {
        title: zh ? "权限总阀门更新失败" : "Failed to update permission ceiling",
        kind: "error",
        duration: 9000,
      });
    } finally {
      permissionBusy = false;
    }
  }

  onMount(() => {
    void refresh();
    if ($developerMode) void loadCompactPolicy();
  });
</script>

<section class="page-scroll mn-settings-page">
  <header class="page-header">
    <p class="page-kicker">{zh ? "应用设置" : "APPLICATION SETTINGS"}</p>
    <h2 class="page-title">{zh ? "通用" : "General"}</h2>
    <p class="tx-project-path">{zh ? "管理 Mnelyra 的基础设置。" : "Manage Mnelyra basics."}</p>
  </header>

  <div class="page-body mn-settings-stack-v2">
    {#if loading || !general}
      <div class="mn-console-panel mn-settings-loading">{zh ? "读取本机配置…" : "Reading local configuration…"}</div>
    {:else}
      <section class="mn-console-panel mn-settings-section">
        <div class="mn-settings-section-head">
          <div class="mn-settings-icon"><Globe2 size={17} /></div>
          <div>
            <span class="page-kicker">{zh ? "界面" : "INTERFACE"}</span>
            <h3>{zh ? "语言" : "Language"}</h3>
            <p>{zh ? "切换后立即应用到整个应用，并在本机持久化。" : "Applied immediately across the app and saved locally."}</p>
          </div>
        </div>
        <div class="mn-segmented-control" role="group" aria-label={zh ? "界面语言" : "Interface language"}>
          <button type="button" class:active={$uiLocale === "zh-CN"} onclick={() => changeLocale("zh-CN")}>简体中文</button>
          <button type="button" class:active={$uiLocale === "en"} onclick={() => changeLocale("en")}>English</button>
        </div>
        <label class="mn-settings-toggle-row">
          <div>
            <strong>{zh ? "我是开发者" : "Developer mode"}</strong>
            <span>{zh ? "显示 Codex 权限与上下文设置。开启本身不会修改配置。" : "Show Codex permission and context settings. Enabling this does not change them."}</span>
          </div>
          <input type="checkbox" checked={$developerMode} onchange={(event) => void changeDeveloperMode(event.currentTarget.checked)} />
          <i aria-hidden="true"></i>
        </label>

        {#if $developerMode}
          <div class="mn-developer-context-panel">
            <div class="mn-developer-context-head">
              <div>
                <span>PERMISSION CEILING</span>
                <strong>{zh ? "权限总阀门" : "Permission ceiling"}</strong>
                <small>{zh ? "限制 Mnelyra 与 MCP 能达到的最高权限；下游自己的限制仍然有效。" : "Caps the maximum access available to Mnelyra and MCP. Stricter downstream limits still apply."}</small>
              </div>
              <b>{general.permissionCeiling.replaceAll("_", " ").toUpperCase()}</b>
            </div>

            <div class="mn-developer-context-body">
              <div class="mn-permission-ceiling-grid" role="group" aria-label={zh ? "权限总阀门" : "Permission ceiling"}>
                <button
                  type="button"
                  class:active={general.permissionCeiling === "automatic"}
                  disabled={permissionBusy}
                  onclick={() => void changePermissionCeiling("automatic")}
                >
                  <strong>{zh ? "自动" : "Automatic"}</strong>
                </button>
                <button
                  type="button"
                  class:active={general.permissionCeiling === "read_only"}
                  disabled={permissionBusy}
                  onclick={() => void changePermissionCeiling("read_only")}
                >
                  <strong>{zh ? "只读" : "Read only"}</strong>
                </button>
                <button
                  type="button"
                  class:active={general.permissionCeiling === "custom"}
                  disabled={permissionBusy}
                  onclick={() => void changePermissionCeiling("custom")}
                >
                  <strong>{zh ? "工作区读写" : "Workspace read/write"}</strong>
                </button>
              </div>
            </div>
          </div>

          <div class="mn-developer-context-panel">
            <div class="mn-developer-context-head">
              <div>
                <span>CODEX CONTEXT</span>
                <strong>{zh ? "上下文与总结" : "Context and compaction"}</strong>
              </div>
              <b>{compactPolicyLoading ? (zh ? "读取中" : "LOADING") : contextMode === "auto" ? "AUTO" : contextMode === "one_million" ? "1M" : "CUSTOM"}</b>
            </div>

            <div class="mn-developer-context-body">
              <div class="mn-compact-mode-switch" role="group" aria-label={zh ? "Codex 上下文模式" : "Codex context mode"}>
                <button
                  type="button"
                  class:active={contextMode === "auto"}
                  disabled={compactPolicyLoading || compactBusy}
                  onclick={() => void chooseContextMode("auto")}
                >{zh ? "自动" : "Automatic"}</button>
                <button
                  type="button"
                  class:active={contextMode === "one_million"}
                  disabled={compactPolicyLoading || compactBusy}
                  onclick={() => void chooseContextMode("one_million")}
                >1M</button>
                <button
                  type="button"
                  class:active={contextMode === "custom"}
                  disabled={compactPolicyLoading || compactBusy}
                  onclick={() => void chooseContextMode("custom")}
                >{zh ? "自定义" : "Custom"}</button>
              </div>

              <p class="mn-compact-auto-copy">
                {contextMode === "auto"
                  ? (zh ? "交给 Codex。上下文使用当前模型默认值，自动总结按 Codex 默认机制触发。" : "Handled by Codex. Context uses the current model default and compaction follows Codex defaults.")
                  : contextMode === "one_million"
                    ? (zh ? "使用 1,000,000 token 上下文，900,000 token 开始自动总结。有效上限由当前模型和通道决定。" : "Uses a 1,000,000-token context window and starts compaction at 900,000 tokens. The active model and route set the effective limit.")
                    : (zh ? "按下方数值写入 Codex 配置，有效上限由当前模型和通道决定。" : "Writes the values below to Codex configuration. The active model and route set the effective limit.")}
              </p>

              {#if contextMode === "custom"}
                <div class="mn-compact-custom-row">
                  <label>
                    <span>{zh ? "上下文窗口" : "Context window"}</span>
                    <input type="number" min="16384" step="1024" placeholder="1000000" bind:value={contextWindowInput} disabled={compactBusy || compactPolicyLoading} />
                  </label>
                  <label>
                    <span>{zh ? "自动总结阈值" : "Auto-compaction threshold"}</span>
                    <input type="number" min="16384" step="1024" placeholder="900000" bind:value={compactLimitInput} disabled={compactBusy || compactPolicyLoading} />
                  </label>
                  <button type="button" class="tx-btn-primary" disabled={compactBusy || compactPolicyLoading} onclick={() => void saveCustomContextPolicy()}>
                    <Save size={13} /> {compactBusy ? (zh ? "保存中…" : "Saving…") : (zh ? "保存" : "Save")}
                  </button>
                </div>
              {/if}
            </div>
          </div>
        {/if}
      </section>

      <section class="mn-console-panel mn-settings-section">
        <div class="mn-settings-section-head">
          <div class="mn-settings-icon"><TerminalSquare size={17} /></div>
          <div>
            <span class="page-kicker">LOCAL RUNTIME</span>
            <h3>{zh ? "本机服务" : "Local services"}</h3>
          </div>
        </div>

        <div class="mn-port-grid">
          <label>
            <span>MCP</span>
            <strong>{zh ? "本地端口" : "Local port"}</strong>
            <input type="number" min="1024" max="65535" value={general.mcpRuntime.local_port} oninput={(event) => updateMcpRuntime("local_port", Number(event.currentTarget.value))} />
            <small>127.0.0.1:{general.mcpRuntime.local_port}/mcp</small>
          </label>
        </div>

        <div class="mn-settings-save-row">
          <span>{zh ? "修改端口会触发对应 runtime 的事务式重绑。" : "Port changes trigger transactional runtime rebinding."}</span>
          <button type="button" class="tx-btn-primary" disabled={saving} onclick={() => void savePorts()}><Save size={13} /> {saving ? (zh ? "保存中…" : "Saving…") : (zh ? "保存设置" : "Save settings")}</button>
        </div>

        <div class="mt-6 border-t border-[var(--color-border)] pt-5">
          <div class="mb-4 flex items-end justify-between gap-4">
            <div>
              <span class="page-kicker">{zh ? "运行诊断" : "RUNTIME DIAGNOSTICS"}</span>
              <h3 class="mt-1">{zh ? "日志与健康" : "Logs and health"}</h3>
              {#if activeProfile}
                <p class="mt-1 text-sm text-[var(--color-text-muted)]">
                  {zh ? "当前活动工作区" : "Active workspace"} · {workspaceRootName(activeProfile.path)}
                </p>
              {/if}
            </div>
          </div>

          <Tabs
            items={diagnosticsTabs}
            value={diagnosticsTab}
            onchange={(value) => (diagnosticsTab = value as DiagnosticsTab)}
          />

          <div class="mt-4">
            {#if $activeWorkspaceState.workspaceId}
              {#key $activeWorkspaceState.workspaceId}
                {#if diagnosticsTab === "logs"}
                  <LogViewer workspaceId={$activeWorkspaceState.workspaceId} service="mcp" />
                {:else}
                  <HealthPanel workspaceId={$activeWorkspaceState.workspaceId} />
                {/if}
              {/key}
            {:else}
              <div class="tx-card p-4 text-sm text-[var(--color-text-muted)]">
                {zh ? "先从左侧选择一个工作区，再查看当前 MCP runtime 的日志与健康状态。" : "Select a workspace first to inspect the active MCP runtime logs and health."}
              </div>
            {/if}
          </div>
        </div>
      </section>

    {/if}
  </div>
</section>
