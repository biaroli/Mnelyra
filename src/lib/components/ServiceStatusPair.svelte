<script lang="ts">
  import StatusOrb from "$lib/components/StatusOrb.svelte";
  import type { RuntimeState } from "$lib/types";

  interface Props {
    mcp: RuntimeState;
    actions: RuntimeState;
  }

  let { mcp, actions }: Props = $props();

  let aggregate = $derived.by<RuntimeState>(() => {
    if (mcp === "error" || actions === "error") return "error";
    if (
      mcp === "running" ||
      mcp === "starting" ||
      mcp === "stopping" ||
      actions === "running" ||
      actions === "starting" ||
      actions === "stopping"
    ) {
      return "running";
    }
    return "stopped";
  });

  let label = $derived(
    aggregate === "error" ? "异常" : aggregate === "running" ? "运行中" : "已关闭",
  );
</script>

<span class="inline-flex items-center" title={label} aria-label={label}>
  <StatusOrb state={aggregate} />
</span>
