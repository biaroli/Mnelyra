<script lang="ts">
  import { goto } from "$app/navigation";
  import { page } from "$app/stores";
  import HealthPanel from "$lib/components/HealthPanel.svelte";
  import LogViewer from "$lib/components/LogViewer.svelte";
  import ServicePanel from "$lib/components/ServicePanel.svelte";
  import StatusOrb from "$lib/components/StatusOrb.svelte";
  import Tabs from "$lib/components/Tabs.svelte";
  import {
    getActionsRuntimeStatus,
    getRuntimeStatus,
    listWorkspaces,
  } from "$lib/api/workspaces";
  import { getGlobalGeneral, setLastWorkspace } from "$lib/api/settings";
  import { actionsRuntimeStates, mcpRuntimeStates, workspaces } from "$lib/stores/app";
  import {
    actionsLocalEndpoint,
    mcpLocalEndpoint,
    type GlobalGeneralConfig,
    type RuntimeState,
    type WorkspaceProfile,
    workspaceRootName,
  } from "$lib/types";

  type ServiceTab = "mcp" | "actions";
  type DetailTab = "overview" | "logs" | "health";

  let profile = $state<WorkspaceProfile | null>(null);
  let general = $state<GlobalGeneralConfig | null>(null);
  let mcpStatus = $state<RuntimeState>("stopped");
  let actionsStatus = $state<RuntimeState>("stopped");
  let mcpMessage = $state("");
  let actionsMessage = $state("");
  let mcpLocal = $state("");
  let mcpPublic = $state("");
  let actionsLocal = $state("");
  let actionsPublic = $state("");
  let activeService = $state<ServiceTab>("mcp");
  let detailTab = $state<DetailTab>("overview");
  let loadGeneration = 0;

  const workspaceId = $derived($page.params.id);
  const detailTabs = [
    { value: "overview", label: "概览" },
    { value: "logs", label: "日志" },
    { value: "health", label: "健康" },
  ];

  function stateLabel(state: RuntimeState): string {
    switch (state) {
      case "running":
        return "运行中";
      case "starting":
        return "启动中";
      case "stopping":
        return "停止中";
      case "error":
        return "错误";
      default:
        return "未运行";
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

    // Selecting a workspace is the switch operation: the backend stops any old
    // MCP root and activates this project with the same global configuration.
    await setLastWorkspace(id);
    if (generation !== loadGeneration || id !== workspaceId) return;

    const [mcp, actions] = await Promise.all([
      getRuntimeStatus(id),
      getActionsRuntimeStatus(id),
    ]);
    if (generation !== loadGeneration || id !== workspaceId) return;

    mcpStatus = mcp.state;
    mcpMessage = mcp.localMessage ?? "";
    mcpLocal = mcp.localEndpoint;
    mcpPublic = mcp.publicEndpoint;
    actionsStatus = actions.state;
    actionsMessage = actions.localMessage ?? "";
    actionsLocal = actions.localEndpoint;
    actionsPublic = actions.publicEndpoint;
    mcpRuntimeStates.update((current) => ({ ...current, [id]: mcp.state }));
    actionsRuntimeStates.update((current) => ({ ...current, [id]: actions.state }));
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
        <p class="page-kicker">PROJECT ROOT</p>
        <h2 class="page-title">{workspaceRootName(profile.path)}</h2>
        <p class="tx-project-path" title={profile.path}>{profile.path}</p>
      </div>

      <div class="mt-4 flex flex-wrap items-center gap-2">
        <button
          type="button"
          class="tx-status-pill"
          class:active={activeService === "mcp"}
          onclick={() => (activeService = "mcp")}
        >
          <StatusOrb state={mcpStatus} />
          <span class="font-medium">MCP</span>
          <span class="text-[var(--color-text-muted)]">{stateLabel(mcpStatus)}</span>
        </button>
        <button
          type="button"
          class="tx-status-pill"
          class:active={activeService === "actions"}
          onclick={() => (activeService = "actions")}
        >
          <StatusOrb state={actionsStatus} />
          <span class="font-medium">Actions</span>
          <span class="text-[var(--color-text-muted)]">{stateLabel(actionsStatus)}</span>
        </button>
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
        {#if activeService === "mcp"}
          <ServicePanel
            title="MCP"
            subtitle="当前项目根目录 Streamable HTTP 工具运行时"
            status={mcpStatus}
            statusMessage={mcpMessage}
            port={general.mcpRuntime.local_port}
            portEditable={false}
            tunnelType={general.mcpTunnel.type}
            localEndpoint={mcpLocal || mcpLocalEndpoint(general.mcpRuntime.local_port)}
            publicEndpoint={mcpPublic}
            publicLabel="公网 MCP"
            showToggle={false}
          />
        {:else}
          <ServicePanel
            title="Actions"
            subtitle="当前项目根目录 OpenAPI 网关"
            status={actionsStatus}
            statusMessage={actionsMessage}
            port={general.actions.local_port}
            portEditable={false}
            tunnelType={general.actions.tunnel_type}
            localEndpoint={actionsLocal || actionsLocalEndpoint(general.actions.local_port)}
            publicEndpoint={actionsPublic}
            publicLabel="公网 Actions"
            showToggle={false}
          />
        {/if}

        <div class="tx-card mt-4 p-4">
          <p class="tx-section-label">运行模型</p>
          <p class="mt-2 text-sm text-[var(--color-text-muted)]">
            端口、隧道、认证与执行策略均来自全局设置。切换左侧工作区时只切换 MCP 的代码根目录，不生成新的 OAuth Client ID，也不复制一套服务配置。
          </p>
        </div>
      {:else if detailTab === "logs"}
        <LogViewer workspaceId={workspaceId!} service={activeService} />
      {:else}
        <HealthPanel workspaceId={workspaceId!} />
      {/if}
    </div>
  </section>
{/if}
