<script lang="ts">
  import "../mnelyra.css";
  import { onMount, untrack } from "svelte";
  import { goto } from "$app/navigation";
  import { page } from "$app/stores";
  import { confirm, open } from "@tauri-apps/plugin-dialog";
  import {
    Cable,
    FolderOpen,
    FolderPlus,
    KeyRound,
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
    getRuntimeStatus,
    listWorkspaces,
  } from "$lib/api/workspaces";
  import { activateWorkspace, getActiveWorkspaceState, getWorkspaceActivity } from "$lib/api/activity";
  import { getProviderStatus } from "$lib/api/providers";
  import { getLastWorkspaceId } from "$lib/api/settings";
  import {
    activeWorkspaceState,
    mcpRuntimeStates,
    providerStatuses,
    workspaceActivity,
    workspaces,
  } from "$lib/stores/app";
  import { showToast } from "$lib/stores/toast";
  import { startUiMemoryGuard } from "$lib/ui-memory-guard";
  import { startCloseGuard } from "$lib/close-guard";
  import { startBackgroundServices } from "$lib/api/startup";
  import CloseConfirmDialog from "$lib/components/CloseConfirmDialog.svelte";
  import {
    checkForUpdates,
    installAvailableUpdate,
    updateState,
  } from "$lib/stores/update";
  import { uiLocale } from "$lib/stores/locale";
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
  let authorityRefreshInFlight = false;
  let authorityRefreshGeneration = 0;
  let providerRefreshInFlight = false;
  let workspaceSwitchGeneration = 0;
  let switchingWorkspaceId = $state<string | null>(null);
  const zh = $derived($uiLocale === "zh-CN");

  async function refreshAuthority() {
    if (authorityRefreshInFlight || switchingWorkspaceId) return;
    const generation = authorityRefreshGeneration;
    authorityRefreshInFlight = true;
    try {
      const active = await getActiveWorkspaceState();
      if (generation !== authorityRefreshGeneration || switchingWorkspaceId) return;
      activeWorkspaceState.set(active);
      const ids = new Set<string>();
      if (active.workspaceId) ids.add(active.workspaceId);
      const selected = $page.params.id;
      if (selected) ids.add(selected);
      if (ids.size === 0) return;
      const entries = await Promise.all(
        [...ids].map(async (id) => [id, await getWorkspaceActivity(id)] as const),
      );
      if (generation !== authorityRefreshGeneration || switchingWorkspaceId) return;
      workspaceActivity.update((current) => {
        const next = { ...current };
        for (const [id, activity] of entries) next[id] = activity;
        return next;
      });
    } catch {
      // Keep the last proven backend snapshot across transient IPC read failures.
    } finally {
      authorityRefreshInFlight = false;
    }
  }

  $effect(() => {
    const id = $page.params.id;
    if ($page.url.pathname.startsWith("/workspace/") && id) {
      untrack(() => void activateRouteWorkspace(id));
    }
  });

  async function activateRouteWorkspace(id: string) {
    const generation = ++workspaceSwitchGeneration;
    authorityRefreshGeneration += 1;
    switchingWorkspaceId = id;

    try {
      if ($activeWorkspaceState.workspaceId !== id) {
        const active = await activateWorkspace(id);
        if (generation !== workspaceSwitchGeneration || $page.params.id !== id) return;
        activeWorkspaceState.set(active);
      }

      const [activity, mcp] = await Promise.all([
        getWorkspaceActivity(id),
        getRuntimeStatus(id),
      ]);
      if (generation !== workspaceSwitchGeneration || $page.params.id !== id) return;
      workspaceActivity.update((current) => ({ ...current, [id]: activity }));
      mcpRuntimeStates.update((current) => ({ ...current, [id]: mcp.state }));
    } catch (error) {
      if (generation !== workspaceSwitchGeneration || $page.params.id !== id) return;
      showToast(String(error), {
        title: zh ? "工作区切换未完成" : "Workspace switch did not complete",
        kind: "error",
        duration: 9000,
      });

      try {
        const active = await getActiveWorkspaceState();
        if (
          generation === workspaceSwitchGeneration
          && $page.params.id === id
          && active.workspaceId
          && active.workspaceId !== id
        ) {
          activeWorkspaceState.set(active);
          await goto(`/workspace/${active.workspaceId}`, { replaceState: true });
        }
      } catch {
        // Keep the target route visible if the backend cannot provide a rollback snapshot.
      }
    } finally {
      if (generation === workspaceSwitchGeneration) {
        switchingWorkspaceId = null;
        authorityRefreshGeneration += 1;
        void refreshAuthority();
      }
    }
  }

  function openConnectors() {
    goto("/connectors");
  }

  async function refreshProviders() {
    if (providerRefreshInFlight) return;
    providerRefreshInFlight = true;
    try {
      const codex = await getProviderStatus("codex");
      providerStatuses.update((current) => ({ ...current, [codex.providerId]: codex }));
    } catch {
      // Provider discovery is optional; Mnelyra remains healthy without a provider runtime.
    } finally {
      providerRefreshInFlight = false;
    }
  }

  async function refreshWorkspaces() {
    const items = await listWorkspaces();
    workspaces.set(items);

    const mcpStates: Record<string, RuntimeState> = {};
    const activityStates: Record<string, Awaited<ReturnType<typeof getWorkspaceActivity>>> = {};
    await Promise.all(
      items.map(async (item) => {
        try {
          const [mcp, activity] = await Promise.all([
            getRuntimeStatus(item.id),
            getWorkspaceActivity(item.id),
          ]);
          mcpStates[item.id] = mcp.state;
          activityStates[item.id] = activity;
        } catch {
          mcpStates[item.id] = "stopped";
        }
      }),
    );
    mcpRuntimeStates.set(mcpStates);
    workspaceActivity.set(activityStates);
    try {
      activeWorkspaceState.set(await getActiveWorkspaceState());
    } catch {
      // Keep the last known backend snapshot if a transient IPC read fails.
    }
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
      await openWorkspace(profile.id);
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
      `确定从 Mnelyra 中移除「${workspaceRootName(workspace.path)}」？\n\n不会删除磁盘中的项目文件。`,
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
      if (
        !$activeWorkspaceState.workspaceId
        || !$workspaces.some((item) => item.id === $activeWorkspaceState.workspaceId)
      ) {
        const next = $workspaces[0];
        if (next) await openWorkspace(next.id);
      }
    } catch (error) {
      showToast(String(error), { title: "移除工作区失败", kind: "error", duration: 8000 });
    }
  }

  async function openWorkspace(id: string) {
    try {
      if ($page.params.id === id && $page.url.pathname === `/workspace/${id}`) return;
      await goto(`/workspace/${id}`);
    } catch (error) {
      showToast(String(error), {
        title: zh ? "工作区切换未完成" : "Workspace switch did not complete",
        kind: "error",
        duration: 9000,
      });
    }
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

  async function checkStartupUpdate() {
    const state = await checkForUpdates();
    if (state.phase !== "available" || !state.latestVersion) return;

    const accepted = await confirm(
      zh
        ? `发现 Mnelyra v${state.latestVersion}。现在下载并安装更新？安装完成后会自动重启。`
        : `Mnelyra v${state.latestVersion} is available. Download and install it now? Mnelyra will restart when installation finishes.`,
      {
        title: zh ? "Mnelyra 更新" : "Mnelyra Update",
        kind: "info",
        okLabel: zh ? "立即更新" : "Update now",
        cancelLabel: zh ? "稍后" : "Later",
      },
    );
    if (!accepted) return;

    try {
      await installAvailableUpdate();
    } catch (error) {
      showToast(String(error), {
        title: zh ? "更新失败" : "Update failed",
        kind: "error",
        duration: 8000,
      });
    }
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
        if ($activeWorkspaceState.workspaceId) {
          await goto(`/workspace/${$activeWorkspaceState.workspaceId}`, { replaceState: true });
        } else {
          const lastId = await getLastWorkspaceId();
          if (lastId && $workspaces.some((item) => item.id === lastId)) {
            await openWorkspace(lastId);
          } else if ($workspaces.length > 0) {
            await openWorkspace($workspaces[0].id);
          }
        }
      }
    })();
    let startupFrame = window.requestAnimationFrame(() => {
      const boot = document.getElementById("mnelyra-boot");
      if (boot) boot.dataset.ready = "true";
      startupFrame = window.requestAnimationFrame(() => {
        window.setTimeout(() => boot?.remove(), 140);
        void startBackgroundServices();
        window.setTimeout(() => void refreshProviders(), 450);
        window.setTimeout(() => void checkStartupUpdate(), 2200);
      });
    });
    const authorityTimer = window.setInterval(() => {
      void refreshAuthority();
    }, 1200);
    const providerTimer = window.setInterval(() => {
      void refreshProviders();
    }, 3000);
    return () => {
      window.cancelAnimationFrame(startupFrame);
      window.clearInterval(authorityTimer);
      window.clearInterval(providerTimer);
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
      <span>{zh ? "通用" : "General"}</span>
    </button>
    <button
      type="button"
      class="tx-settings-link {$page.url.pathname === '/connectors' ? 'active' : ''}"
      onclick={openConnectors}
    >
      <Cable size={14} strokeWidth={1.8} />
      <span>{zh ? "连接" : "Connections"}</span>
    </button>
    <button
      type="button"
      class="tx-settings-link {$page.url.pathname === '/settings/keys' ? 'active' : ''}"
      onclick={openKeysSettings}
    >
      <KeyRound size={14} strokeWidth={1.8} />
      <span>{zh ? "认证" : "Authentication"}</span>
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
      <span>{zh ? "更新" : "Updates"}</span>
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
          selected={$page.params.id === workspace.id && $page.url.pathname.startsWith('/workspace/')}
          activeRoot={$activeWorkspaceState.workspaceId === workspace.id}
          activity={$workspaceActivity[workspace.id]}
          mcpState={$mcpRuntimeStates[workspace.id] ?? "stopped"}
          onClick={() => void openWorkspace(workspace.id)}
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
          <p class="tx-modal__kicker">{zh ? "本地工作区" : "LOCAL WORKSPACE"}</p>
          <h2 id="add-workspace-title" class="tx-modal__title">{zh ? "打开项目" : "Open project"}</h2>
        </div>
        <button
          type="button"
          class="tx-icon-button"
          aria-label={zh ? "关闭" : "Close"}
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
          <span class="tx-label">{zh ? "项目根目录" : "Project root"}</span>
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
          {addingWorkspace ? (zh ? "打开中…" : "Opening…") : (zh ? "打开" : "Open")}
        </button>
      </form>

      <div class="tx-modal__divider"><span>{zh ? "或" : "OR"}</span></div>

      <button
        type="button"
        class="tx-folder-picker"
        disabled={addingWorkspace}
        onclick={() => void chooseWorkspaceFolder()}
      >
        <span class="tx-folder-picker__icon"><FolderOpen size={19} strokeWidth={1.8} /></span>
        <span class="min-w-0 flex-1 text-left">
          <strong>{zh ? "从文件夹选择" : "Choose a folder"}</strong>
          <small>{zh ? "在系统文件选择器中定位项目根目录" : "Locate the project root with the system folder picker"}</small>
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
