<script lang="ts">
  import { goto } from "$app/navigation";
  import { page } from "$app/stores";
  import { activateWorkspace } from "$lib/api/activity";
  import { listWorkspaces } from "$lib/api/workspaces";
  import WorkspaceMemoryView from "$lib/components/WorkspaceMemoryView.svelte";
  import { activeWorkspaceState, workspaces } from "$lib/stores/app";
  import { uiLocale } from "$lib/stores/locale";
  import { showToast } from "$lib/stores/toast";
  import type { WorkspaceProfile } from "$lib/types";

  let profile = $state<WorkspaceProfile | null>(null);
  let loadGeneration = 0;
  const zh = $derived($uiLocale === "zh-CN");

  async function load(id: string, generation: number) {
    try {
      let items = $workspaces;
      if (!items.some((item) => item.id === id)) {
        items = await listWorkspaces();
        if (generation !== loadGeneration) return;
        workspaces.set(items);
      }
      const nextProfile = items.find((item) => item.id === id) ?? null;
      if (!nextProfile) {
        await goto("/", { replaceState: true });
        return;
      }

      if ($activeWorkspaceState.workspaceId !== id) {
        activeWorkspaceState.set(await activateWorkspace(id));
        if (generation !== loadGeneration) return;
      }
      profile = nextProfile;
    } catch (error) {
      if (generation !== loadGeneration) return;
      showToast(String(error), {
        title: zh ? "项目切换未完成" : "Project switch did not complete",
        kind: "error",
        duration: 9000,
      });
    }
  }

  $effect(() => {
    const id = $page.params.id;
    profile = null;
    const generation = ++loadGeneration;
    if (id) void load(id, generation);
  });
</script>

{#if profile}
  {#key profile.id}
    <WorkspaceMemoryView workspaceId={profile.id} workspacePath={profile.path} />
  {/key}
{/if}
