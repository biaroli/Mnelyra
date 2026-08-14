<script lang="ts">
  import ServiceStatusPair from "$lib/components/ServiceStatusPair.svelte";
  import { workspaceRootName, type RuntimeState, type WorkspaceProfile } from "$lib/types";
  import { Folder } from "@lucide/svelte";

  interface Props {
    workspace: WorkspaceProfile;
    active: boolean;
    mcpState: RuntimeState;
    actionsState: RuntimeState;
    onClick: () => void;
    onContextMenu?: (event: MouseEvent) => void;
  }

  let { workspace, active, mcpState, actionsState, onClick, onContextMenu }: Props = $props();
</script>

<div class="tx-nav-item" class:active>
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
    <span class="tx-node-name">{workspaceRootName(workspace.path)}</span>
    <span class="tx-node-status"><ServiceStatusPair mcp={mcpState} actions={actionsState} /></span>
  </button>
</div>
