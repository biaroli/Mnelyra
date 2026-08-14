<script lang="ts">
  import "../app.css";
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { page } from "$app/stores";
  import { confirm, open } from "@tauri-apps/plugin-dialog";
  import {
    FolderOpen,
    FolderPlus,
    KeyRound,
    RadioTower,
    RefreshCw,
    SlidersHorizontal,
    Trash2,
    X,
  } from "@lucide/svelte";
  import AppShell from "$lib/components/AppShell.svelte";
  import ToastHost from "$lib/components/ToastHost.svelte";
  import WorkspaceNavItem from "$lib/components/WorkspaceNavItem.svelte";
  import {
    createWorkspace,
    deleteWorkspace,
    getActionsRuntimeStatus,
    getRuntimeStatus,
    listWorkspaces,
  } from "$lib/api/workspaces";
  import { getLastWorkspaceId } from "$lib/api/settings";
  import { actionsRuntimeStates, mcpRuntimeStates, workspaces } from "$lib/stores/app";
  import { showToast } from "$lib/stores/toast";
  import { startUiMemoryGuard } from "$lib/ui-memory-guard";
  import { startCloseGuard } from "$lib/close-guard";
  import CloseConfirmDialog from "$lib/components/CloseConfirmDialog.svelte";
  import { checkForUpdates, updateState } from "$lib/stores/update";
  import type { RuntimeState } from "$lib/types";
  import { workspaceRootName, type WorkspaceProfile } from "$lib/types";

  let { children } = $props();
  let closeConfirmOpen = $state(false);
  let addWorkspaceOpen = $state(false);
  let manualWorkspacePath = $state("");
  let addingWorkspace = $state(false);
  let contextWorkspace = $state<WorkspaceProfile | null>(null);
  let contextX = $state(0);
  let contextY = $state(0);

  async function refreshWorkspaces() {
    const items = await listWorkspaces();
    workspaces.set(items);

    const mcpStates: Record<string, RuntimeState> = {};
    const actionsStates: Record<string, RuntimeState> = {};
    await Promise.all(
      items.map(async (item) => {
        try {
          const [mcp, actions] = await Promise.all([
            getRuntimeStatus(item.id),
            getActionsRuntimeStatus(item.id),
          ]);
          mcpStates[item.id] = mcp.state;
          actionsStates[item.id] = actions.state;
        } catch {
          mcpStates[item.id] = "stopped";
          actionsStates[item.id] = "stopped";
        }
      }),
    );
    mcpRuntimeStates.set(mcpStates);
    actionsRuntimeStates.set(actionsStates);
  }

  async function createWorkspaceFromPath(path: string) {
    const normalized = path.trim().replace(/^['"]|['"]$/g, "");
    if (!normalized || addingWorkspace) return;
    addingWorkspace = true;
    try {
      const profile = await createWorkspace(normalized);
      await refreshWorkspaces();
      addWorkspaceOpen = false;
      manualWorkspacePath = "";
      goto(`/workspace/${profile.id}`);
    } catch (error) {
      showToast(String(error), {
        title: "添加工作区失败",
        kind: "error",
        duration: 8000,
      });
    } finally {
      addingWorkspace = false;
    }
  }

  function addWorkspace() {
    contextWorkspace = null;
    addWorkspaceOpen = true;
  }

  async function chooseWorkspaceFolder() {
    try {
      const selected = await open({ directory: true, multiple: false });
      if (!selected || Array.isArray(selected)) return;
      await createWorkspaceFromPath(selected);
    } catch (error) {
      showToast(String(error), {
        title: "无法打开目录选择器",
        kind: "error",
        duration: 8000,
      });
    }
  }

  function openWorkspaceContext(workspace: WorkspaceProfile, event: MouseEvent) {
    contextWorkspace = workspace;
    contextX = Math.min(event.clientX, window.innerWidth - 184);
    contextY = Math.min(event.clientY, window.innerHeight - 88);
  }

  async function removeContextWorkspace() {
    const workspace = contextWorkspace;
    contextWorkspace = null;
    if (!workspace) return;
    const confirmed = await confirm(
      `确定从 RootRelay 中移除「${workspaceRootName(workspace.path)}」？\n\n不会删除磁盘中的项目文件。`,
      {
        title: "移除工作区",
        kind: "warning",
        okLabel: "移除",
        cancelLabel: "取消",
      },
    );
    if (!confirmed) return;
    try {
      await deleteWorkspace(workspace.id);
      await refreshWorkspaces();
      if ($page.url.pathname === `/workspace/${workspace.id}`) {
        const next = $workspaces[0];
        await goto(next ? `/workspace/${next.id}` : "/");
      }
    } catch (error) {
      showToast(String(error), { title: "移除工作区失败", kind: "error", duration: 8000 });
    }
  }

  function openWorkspace(id: string) {
    goto(`/workspace/${id}`);
  }

  function openFrpSettings() {
    goto("/settings/frp");
  }

  function openGeneralSettings() {
    goto("/settings/general");
  }

  function openUpdateSettings() {
    goto("/settings/update");
  }

  function openKeysSettings() {
    goto("/settings/keys");
  }

  onMount(() => {
    const stopGuard = startUiMemoryGuard();
    const stopClose = startCloseGuard(() => {
      closeConfirmOpen = true;
    });
    void (async () => {
      await refreshWorkspaces();
      const path = $page.url.pathname;
      if (path === "/") {
        const lastId = await getLastWorkspaceId();
        if (lastId && $workspaces.some((item) => item.id === lastId)) {
          goto(`/workspace/${lastId}`);
        } else if ($workspaces.length > 0) {
          goto(`/workspace/${$workspaces[0].id}`);
        }
      }
    })();
    void checkForUpdates();
    return () => {
      stopGuard();
      stopClose();
    };
  });
</script>

<AppShell onAddWorkspace={addWorkspace}>
  {#snippet settingsNav()}
    <button
      type="button"
      class="tx-settings-link {$page.url.pathname === '/settings/general' ? 'active' : ''}"
      onclick={openGeneralSettings}
    >
      <SlidersHorizontal size={14} strokeWidth={1.8} />
      <span>通用</span>
    </button>
    <button
      type="button"
      class="tx-settings-link {$page.url.pathname === '/settings/keys' ? 'active' : ''}"
      onclick={openKeysSettings}
    >
      <KeyRound size={14} strokeWidth={1.8} />
      <span>认证</span>
    </button>
    <button
      type="button"
      class="tx-settings-link {$page.url.pathname === '/settings/frp' ? 'active' : ''}"
      onclick={openFrpSettings}
    >
      <RadioTower size={14} strokeWidth={1.8} />
      <span>FRP 配置</span>
    </button>
    <button
      type="button"
      class="tx-settings-link {$page.url.pathname === '/settings/update' ? 'active' : ''}"
      onclick={openUpdateSettings}
    >
      <RefreshCw
        size={14}
        strokeWidth={1.8}
        class={$updateState.phase === "checking" ? "animate-spin" : ""}
      />
      <span>更新</span>
      {#if $updateState.phase === "available"}
        <span
          class="ml-auto h-1.5 w-1.5 rounded-full bg-[var(--color-accent)] shadow-[0_0_8px_var(--color-accent)]"
          title={`发现 v${$updateState.latestVersion ?? "新版本"}`}
          aria-label="有可用更新"
        ></span>
      {/if}
    </button>
  {/snippet}
  {#snippet sidebar()}
    <div class="space-y-1">
      {#each $workspaces as workspace (workspace.id)}
        <WorkspaceNavItem
          workspace={workspace}
          active={$page.url.pathname === `/workspace/${workspace.id}`}
          mcpState={$mcpRuntimeStates[workspace.id] ?? "stopped"}
          actionsState={$actionsRuntimeStates[workspace.id] ?? "stopped"}
          onClick={() => openWorkspace(workspace.id)}
          onContextMenu={(event) => openWorkspaceContext(workspace, event)}
        />
      {/each}
    </div>
  {/snippet}

  {#snippet children()}
    {@render children()}
  {/snippet}
</AppShell>

{#if addWorkspaceOpen}
  <div
    class="tx-modal-overlay"
    role="presentation"
    onclick={(event) => {
      if (event.currentTarget === event.target && !addingWorkspace) addWorkspaceOpen = false;
    }}
  >
    <dialog class="tx-modal" open aria-labelledby="add-workspace-title">
      <div class="tx-modal__head">
        <div>
          <p class="tx-modal__kicker">OPEN LOCAL ROOT</p>
          <h2 id="add-workspace-title" class="tx-modal__title">打开项目</h2>
        </div>
        <button
          type="button"
          class="tx-icon-button"
          aria-label="关闭"
          disabled={addingWorkspace}
          onclick={() => (addWorkspaceOpen = false)}
        >
          <X size={16} />
        </button>
      </div>

      <form
        class="tx-path-entry"
        onsubmit={(event) => {
          event.preventDefault();
          void createWorkspaceFromPath(manualWorkspacePath);
        }}
      >
        <label class="tx-field flex-1">
          <span class="tx-label">项目根目录</span>
          <input
            class="tx-input tx-mono"
            type="text"
            placeholder="E:\\Projects\\my-project"
            bind:value={manualWorkspacePath}
          />
        </label>
        <button
          type="submit"
          class="tx-btn-primary tx-path-entry__submit"
          disabled={addingWorkspace || !manualWorkspacePath.trim()}
        >
          <FolderPlus size={15} />
          {addingWorkspace ? "打开中…" : "打开"}
        </button>
      </form>

      <div class="tx-modal__divider"><span>或</span></div>

      <button
        type="button"
        class="tx-folder-picker"
        disabled={addingWorkspace}
        onclick={() => void chooseWorkspaceFolder()}
      >
        <span class="tx-folder-picker__icon"><FolderOpen size={19} strokeWidth={1.8} /></span>
        <span class="min-w-0 flex-1 text-left">
          <strong>从文件夹选择</strong>
          <small>在系统文件选择器中定位项目根目录</small>
        </span>
        <span class="tx-folder-picker__arrow">↗</span>
      </button>
    </dialog>
  </div>
{/if}

{#if contextWorkspace}
  <button
    type="button"
    class="tx-context-scrim"
    aria-label="关闭工作区菜单"
    onclick={() => (contextWorkspace = null)}
  ></button>
  <div class="tx-context-menu" style={`left:${contextX}px;top:${contextY}px`}>
    <button type="button" class="tx-context-danger" onclick={() => void removeContextWorkspace()}>
      <Trash2 size={14} />
      <span>移除工作区</span>
    </button>
  </div>
{/if}

<ToastHost />
<CloseConfirmDialog
  open={closeConfirmOpen}
  onCancel={() => {
    closeConfirmOpen = false;
  }}
/>
