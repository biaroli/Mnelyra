<script lang="ts">
  import { goto } from "$app/navigation";
  import { page } from "$app/stores";
  import HealthPanel from "$lib/components/HealthPanel.svelte";
  import LogViewer from "$lib/components/LogViewer.svelte";
  import ServicePanel from "$lib/components/ServicePanel.svelte";
  import StatusOrb from "$lib/components/StatusOrb.svelte";
  import Tabs from "$lib/components/Tabs.svelte";
  import {
    getRuntimeStatus,
    listWorkspaces,
  } from "$lib/api/workspaces";
  import { activateWorkspace, getWorkspaceActivity } from "$lib/api/activity";
  import { getGlobalGeneral } from "$lib/api/settings";
  import {
    activeWorkspaceState,
    mcpRuntimeStates,
    workspaceActivity,
    workspaces,
  } from "$lib/stores/app";
  import { uiLocale } from "$lib/stores/locale";
  import { showToast } from "$lib/stores/toast";
  import {
    mcpLocalEndpoint,
    type GlobalGeneralConfig,
    type RuntimeState,
    type WorkspaceProfile,
    workspaceRootName,
  } from "$lib/types";

  type DetailTab = "overview" | "logs" | "health";

  let profile = $state<WorkspaceProfile | null>(null);
  let general = $state<GlobalGeneralConfig | null>(null);
  let mcpStatus = $state<RuntimeState>("stopped");
  let mcpMessage = $state("");
  let mcpLocal = $state("");
  let mcpPublic = $state("");
  let detailTab = $state<DetailTab>("overview");
  let loadGeneration = 0;
  const zh = $derived($uiLocale === "zh-CN");

  const workspaceId = $derived($page.params.id);
  const detailTabs = $derived([
    { value: "overview", label: zh ? "概览" : "Overview" },
    { value: "logs", label: zh ? "日志" : "Logs" },
    { value: "health", label: zh ? "健康" : "Health" },
  ]);

  function stateLabel(state: RuntimeState): string {
    switch (state) {
      case "running":
        return zh ? "运行中" : "Running";
      case "starting":
        return zh ? "启动中" : "Starting";
      case "stopping":
        return zh ? "停止中" : "Stopping";
      case "error":
        return zh ? "错误" : "Error";
      default:
        return zh ? "未运行" : "Stopped";
    }
  }

  async function load(id = workspaceId) {
    if (!id) return;
    const generation = ++loadGeneration;
    const [items, nextGeneral] = await Promise.all([listWorkspaces(), getGlobalGeneral()]);
    if (generation !== loadGeneration || id !== workspaceId) return;

    workspaces.set(items);
    const nextProfile = items.find((item) => item.id === id) ?? null;
    if (!nextProfile) {
      await goto("/");
      return;
    }

    profile = nextProfile;
    general = nextGeneral;

    // Route selection is only UI selection. The backend transaction below owns
    // authority and may keep the old root active if drain/start/verification fails.
    try {
      const active = await activateWorkspace(id);
      activeWorkspaceState.set(active);
      const activity = await getWorkspaceActivity(id);
      workspaceActivity.update((current) => ({ ...current, [id]: activity }));
    } catch (error) {
      showToast(String(error), {
        title: zh ? "工作区切换未完成" : "Workspace switch did not complete",
        kind: "error",
        duration: 9000,
      });
    }
    if (generation !== loadGeneration || id !== workspaceId) return;

    const mcp = await getRuntimeStatus(id);
    if (generation !== loadGeneration || id !== workspaceId) return;

    mcpStatus = mcp.state;
    mcpMessage = mcp.localMessage ?? "";
    mcpLocal = mcp.localEndpoint;
    mcpPublic = mcp.publicEndpoint;
    mcpRuntimeStates.update((current) => ({ ...current, [id]: mcp.state }));
  }

  $effect(() => {
    const id = workspaceId;
    if (!id) return;
    profile = null;
    general = null;
    void load(id);
    return () => {
      loadGeneration += 1;
    };
  });
</script>

{#if profile && general}
  <section class="page-scroll">
    <header class="page-header">
      <div>
        <p class="page-kicker">{zh ? "工作区" : "WORKSPACE"}</p>
        <h2 class="page-title">{workspaceRootName(profile.path)}</h2>
        <p class="tx-project-path" title={profile.path}>{profile.path}</p>
      </div>

      <div class="mn-workspace-service-switcher">
        <div class="tx-status-pill active">
          <StatusOrb state={mcpStatus} />
          <span>MCP</span>
          <strong>{stateLabel(mcpStatus)}</strong>
        </div>
      </div>
    </header>

    <div class="page-body">
      <div class="mb-5">
        <Tabs
          items={detailTabs}
          value={detailTab}
          onchange={(value) => (detailTab = value as DetailTab)}
        />
      </div>

      {#if detailTab === "overview"}
        <ServicePanel
          title="MCP"
          subtitle={zh ? "当前工作区的 Streamable HTTP 工具运行时" : "Streamable HTTP tool runtime for this workspace"}
          status={mcpStatus}
          statusMessage={mcpMessage}
          port={general.mcpRuntime.local_port}
          portEditable={false}
          tunnelType={general.mcpTunnel.type}
          localEndpoint={mcpLocal || mcpLocalEndpoint(general.mcpRuntime.local_port)}
          publicEndpoint={mcpPublic}
          publicLabel={zh ? "公网 MCP" : "Public MCP"}
          showToggle={false}
        />
      {:else if detailTab === "logs"}
        <LogViewer workspaceId={workspaceId!} service="mcp" />
      {:else}
        <HealthPanel workspaceId={workspaceId!} />
      {/if}
    </div>
  </section>
{/if}
