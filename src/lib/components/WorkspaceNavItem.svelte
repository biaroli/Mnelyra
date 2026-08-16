<script lang="ts">
  import StatusOrb from "$lib/components/StatusOrb.svelte";
  import {
    workspaceRootName,
    type ActivitySnapshot,
    type RuntimeState,
    type WorkspaceProfile,
  } from "$lib/types";
  import { Folder } from "@lucide/svelte";

  interface Props {
    workspace: WorkspaceProfile;
    selected: boolean;
    activeRoot: boolean;
    activity?: ActivitySnapshot;
    mcpState: RuntimeState;
    onClick: () => void;
    onContextMenu?: (event: MouseEvent) => void;
  }

  let {
    workspace,
    selected,
    activeRoot,
    activity,
    mcpState,
    onClick,
    onContextMenu,
  }: Props = $props();

  const busyLabel = $derived.by(() => {
    if (!activity) return "";
    const parts: string[] = [];
    if (activity.activeMcpRequests > 0) parts.push(`${activity.activeMcpRequests} req`);
    if (activity.runningExecSessions > 0) parts.push(`${activity.runningExecSessions} jobs`);
    if (activity.activeProviderTurns > 0) parts.push(`${activity.activeProviderTurns} turns`);
    return parts.join(" · ");
  });
</script>

<div class="tx-nav-item" class:selected class:active-root={activeRoot}>
  <button
    type="button"
    class="tx-nav-button"
    onclick={onClick}
    oncontextmenu={(event) => {
      event.preventDefault();
      onContextMenu?.(event);
    }}
  >
    <Folder class="tx-node-folder" size={15} strokeWidth={1.7} />
    <span class="tx-node-copy">
      <span class="tx-node-name">{workspaceRootName(workspace.path)}</span>
      {#if busyLabel}
        <span class="tx-node-activity">{busyLabel}</span>
      {/if}
    </span>
    <span class="tx-node-status">
      {#if activeRoot}
        <span class="tx-active-label">ACTIVE</span>
      {/if}
      <StatusOrb state={mcpState} />
    </span>
  </button>
</div>
