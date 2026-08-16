<script lang="ts">
  import EmptyState from "$lib/components/EmptyState.svelte";
  import { workspaces } from "$lib/stores/app";
  import { DEFAULT_SERVICE_PORT } from "$lib/types";
</script>

<div class="page-scroll flex flex-col items-center justify-center p-8">
  {#if $workspaces.length === 0}
    <EmptyState />
    <p class="mt-6 max-w-md text-center text-sm text-[var(--color-text-muted)]">
      MCP 默认端口为 <span class="font-mono">{DEFAULT_SERVICE_PORT}</span>，可在通用设置中修改。
    </p>
  {/if}
</div>
