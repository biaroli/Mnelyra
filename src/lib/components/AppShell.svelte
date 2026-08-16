<script lang="ts">
  import { onMount } from "svelte";
  import { REPO_URL } from "$lib/app-links";
  import { openUrl } from "$lib/api/app-info";
  import { message } from "@tauri-apps/plugin-dialog";
  import { FolderPlus, Github, Settings } from "@lucide/svelte";
  import type { Snippet } from "svelte";
  import { uiLocale } from "$lib/stores/locale";

  interface Props {
    children: Snippet;
    sidebar: Snippet;
    onAddWorkspace?: () => void | Promise<void>;
    primaryNav?: Snippet;
    settingsNav?: Snippet;
  }

  let { children, sidebar, onAddWorkspace, primaryNav, settingsNav }: Props = $props();
  const zh = $derived($uiLocale === "zh-CN");
  let settingsPopover = $state<HTMLDetailsElement | null>(null);

  async function openRepo() {
    try {
      await openUrl(REPO_URL);
    } catch (e) {
      await message(String(e), { title: "无法打开仓库", kind: "error" });
    }
  }

  onMount(() => {
    const closeSettingsOnOutsidePointer = (event: PointerEvent) => {
      if (!settingsPopover?.open) return;
      const target = event.target;
      if (target instanceof Node && !settingsPopover.contains(target)) {
        settingsPopover.removeAttribute("open");
      }
    };

    document.addEventListener("pointerdown", closeSettingsOnOutsidePointer);
    return () => document.removeEventListener("pointerdown", closeSettingsOnOutsidePointer);
  });
</script>

<div class="app-layout">
  <aside class="tx-sidebar" aria-label={zh ? "主导航" : "Primary navigation"}>
    <div class="tx-sidebar-header">
      <div class="tx-brand-row">
        <div class="tx-brand-copy">
          <img class="tx-brand-mark" src="/favicon.png" alt="" aria-hidden="true" />
          <h1 class="tx-brand-title">Mnelyra</h1>
        </div>
      </div>
    </div>

    <div class="tx-sidebar-body">
      {#if primaryNav}
        <div class="tx-workbench-nav">
          <p class="tx-sidebar-section-label">{zh ? "导航" : "Navigation"}</p>
          <div class="tx-settings-stack">
            {@render primaryNav()}
          </div>
        </div>
      {/if}
      <div class="tx-workspace-section">
        <div class="tx-section-head">
          <p class="tx-sidebar-section-label">{zh ? "工作区" : "Workspaces"}</p>
          {#if onAddWorkspace}
            <button type="button" class="tx-add-compact" onclick={onAddWorkspace} aria-label={zh ? "打开项目" : "Open project"} title={zh ? "打开项目" : "Open project"}>
              <FolderPlus size={15} strokeWidth={1.8} />
            </button>
          {/if}
        </div>
        <div class="tx-workspace-scroll">
          {@render sidebar()}
        </div>
      </div>
    </div>

    <div class="tx-sidebar-footer">
      <div class="tx-footer-actions">
        <button type="button" class="tx-repo-link" onclick={() => void openRepo()}>
          <Github size={13} strokeWidth={1.9} />
          <span>GitHub</span>
        </button>

        {#if settingsNav}
          <details class="tx-settings-popover" bind:this={settingsPopover}>
            <summary
              class="tx-footer-icon-button"
              aria-label={zh ? "打开设置" : "Open settings"}
              title={zh ? "设置" : "Settings"}
            >
              <Settings size={14} strokeWidth={1.8} />
            </summary>
            <div class="tx-settings-popover__panel">
              <div class="tx-settings-popover__head">
                <span>{zh ? "设置" : "SETTINGS"}</span>
                <strong>Mnelyra</strong>
              </div>
              <div class="tx-settings-stack">
                {@render settingsNav()}
              </div>
            </div>
          </details>
        {/if}
      </div>
    </div>
  </aside>

  <main class="tx-main">
    {@render children()}
  </main>
</div>

<svelte:head>
  <title>Mnelyra</title>
</svelte:head>
