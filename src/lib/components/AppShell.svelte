<script lang="ts">
  import { APP_VERSION } from "$lib/app-version";
  import { REPO_URL } from "$lib/app-links";
  import { openUrl } from "$lib/api/app-info";
  import { message } from "@tauri-apps/plugin-dialog";
  import { FolderPlus, Github } from "@lucide/svelte";
  import type { Snippet } from "svelte";

  interface Props {
    children: Snippet;
    sidebar: Snippet;
    onAddWorkspace?: () => void | Promise<void>;
    settingsNav?: Snippet;
  }

  let { children, sidebar, onAddWorkspace, settingsNav }: Props = $props();

  async function openRepo() {
    try {
      await openUrl(REPO_URL);
    } catch (e) {
      await message(String(e), { title: "无法打开仓库", kind: "error" });
    }
  }
</script>

<div class="app-layout">
  <aside class="tx-sidebar" aria-label="主导航">
    <div class="tx-sidebar-header">
      <div class="tx-brand-row">
        <div class="tx-brand-copy">
          <span class="tx-brand-dot" aria-hidden="true"></span>
          <h1 class="tx-brand-title">RootRelay</h1>
        </div>
        <span class="tx-app-version">v{APP_VERSION}</span>
      </div>
    </div>

    <div class="tx-sidebar-body">
      <div class="tx-section-head">
        <p class="tx-sidebar-section-label">工作区</p>
        {#if onAddWorkspace}
          <button type="button" class="tx-add-compact" onclick={onAddWorkspace} aria-label="打开项目" title="打开项目">
            <FolderPlus size={15} strokeWidth={1.8} />
          </button>
        {/if}
      </div>
      {@render sidebar()}
    </div>

    <div class="tx-sidebar-footer">
      {#if settingsNav}
        <p class="tx-sidebar-section-label">设置</p>
        <div class="tx-settings-stack">
          {@render settingsNav()}
        </div>
      {/if}
      <button type="button" class="tx-repo-link" onclick={() => void openRepo()}>
        <Github size={13} strokeWidth={1.9} />
        <span>GitHub</span>
      </button>
    </div>
  </aside>

  <main class="tx-main">
    {@render children()}
  </main>
</div>

<svelte:head>
  <title>RootRelay</title>
</svelte:head>
