<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { getActiveWorkspaceState } from "$lib/api/activity";

  onMount(() => {
    void (async () => {
      const active = await getActiveWorkspaceState();
      await goto(active.workspaceId ? `/workspace/${active.workspaceId}` : "/", { replaceState: true });
    })();
  });
</script>
