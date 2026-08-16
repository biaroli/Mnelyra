<script lang="ts">
  import { onMount } from "svelte";
  import { message } from "@tauri-apps/plugin-dialog";
  import { Globe2, Save, TerminalSquare } from "@lucide/svelte";
  import {
    getCodexContextPolicy,
    setCodexAutoCompactLimit,
    setPermissionCeiling,
  } from "$lib/api/providers";
  import { getGlobalGeneral, setGlobalGeneral } from "$lib/api/settings";
  import { setUiLocale, uiLocale, type UiLocale } from "$lib/stores/locale";
  import { developerMode, setDeveloperMode } from "$lib/stores/developer";
  import { showToast } from "$lib/stores/toast";
  import type { CodexConfigReadResponse, GlobalGeneralConfig } from "$lib/types";

  let general = $state<GlobalGeneralConfig | null>(null);
  let loading = $state(true);
  let saving = $state(false);
  let codexPolicy = $state<CodexConfigReadResponse | null>(null);
  let compactPolicyLoading = $state(false);
  let compactBusy = $state(false);
  let permissionBusy = $state(false);
  let compactMode = $state<"auto" | "custom">("auto");
  let compactLimitInput = $state("");
  const zh = $derived($uiLocale === "zh-CN");

  function compactLimitFrom(policy: CodexConfigReadResponse | null): number | null {
    const value = policy?.config?.model_auto_compact_token_limit;
    return typeof value === "number" && Number.isFinite(value) ? value : null;
  }

  const compactLimit = $derived(compactLimitFrom(codexPolicy));

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
      const limit = compactLimitFrom(next);
      codexPolicy = next;
      compactMode = limit == null ? "auto" : "custom";
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

  async function saveCompactLimit(limit: number | null) {
    if (compactBusy) return;
    compactBusy = true;
    try {
      await setCodexAutoCompactLimit(limit);
      await loadCompactPolicy(true);
      showToast(
        limit == null
          ? (zh ? "已恢复 ChatGPT 自动总结。" : "ChatGPT automatic compaction restored.")
          : (zh ? "自定义总结阈值已保存。" : "Custom compaction threshold saved."),
        { title: zh ? "已保存" : "Saved", kind: "success" },
      );
    } catch (error) {
      showToast(String(error), {
        title: zh ? "总结策略保存失败" : "Failed to save compaction policy",
        kind: "error",
        duration: 9000,
      });
    } finally {
      compactBusy = false;
    }
  }

  async function chooseCompactMode(mode: "auto" | "custom") {
    if (mode === "custom") {
      compactMode = "custom";
      return;
    }
    compactMode = "auto";
    if (compactLimit != null) await saveCompactLimit(null);
  }

  async function saveCustomCompactLimit() {
    const value = Number(compactLimitInput);
    if (!Number.isInteger(value) || value < 16_384) {
      showToast(
        zh ? "请输入至少 16,384 的整数 token 阈值。" : "Enter an integer token threshold of at least 16,384.",
        { title: zh ? "无效阈值" : "Invalid threshold", kind: "warning" },
      );
      return;
    }
    await saveCompactLimit(value);
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
            <span>{zh ? "开启后在这里显示 ChatGPT 总结阈值设置。默认保持自动，不会改变现有行为。" : "Show ChatGPT compaction controls here. Automatic remains the default and enabling this does not change current behavior."}</span>
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
                <span>CHATGPT COMPACTION</span>
                <strong>{zh ? "总结阈值" : "Compaction threshold"}</strong>
              </div>
              <b>{compactPolicyLoading ? (zh ? "读取中" : "LOADING") : compactMode === "auto" ? "AUTO" : "CUSTOM"}</b>
            </div>

            <div class="mn-developer-context-body">
              <div class="mn-compact-mode-switch" role="group" aria-label={zh ? "总结阈值模式" : "Compaction threshold mode"}>
                <button
                  type="button"
                  class:active={compactMode === "auto"}
                  disabled={compactPolicyLoading || compactBusy}
                  onclick={() => void chooseCompactMode("auto")}
                >{zh ? "自动" : "Automatic"}</button>
                <button
                  type="button"
                  class:active={compactMode === "custom"}
                  disabled={compactPolicyLoading || compactBusy}
                  onclick={() => void chooseCompactMode("custom")}
                >{zh ? "自定义" : "Custom"}</button>
              </div>

              {#if compactMode === "custom"}
                <div class="mn-compact-custom-row">
                  <label>
                    <span>{zh ? "Token 阈值" : "Token threshold"}</span>
                    <input type="number" min="16384" step="1024" placeholder="256000" bind:value={compactLimitInput} disabled={compactBusy || compactPolicyLoading} />
                    <small>{zh ? "最小 16,384；达到该阈值后 ChatGPT 会触发自动总结。" : "Minimum 16,384; ChatGPT auto-compacts when the threshold is reached."}</small>
                  </label>
                  <button type="button" class="tx-btn-primary" disabled={compactBusy || compactPolicyLoading} onclick={() => void saveCustomCompactLimit()}>
                    <Save size={13} /> {compactBusy ? (zh ? "保存中…" : "Saving…") : (zh ? "保存阈值" : "Save threshold")}
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
      </section>

    {/if}
  </div>
</section>
