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
  {:else}
    <div class="max-w-lg text-center">
      <p class="text-[11px] font-medium uppercase tracking-[0.18em] text-[var(--color-text-muted)]">
        开始
      </p>
      <h2 class="mt-2 text-2xl font-semibold">从左侧选择一个工作区</h2>
      <p class="mt-3 text-sm leading-relaxed text-[var(--color-text-secondary)]">
        选择后 Mnelyra 会把该目录设为当前工作区，并切换对应的本机服务。
      </p>
    </div>
  {/if}
</div>
