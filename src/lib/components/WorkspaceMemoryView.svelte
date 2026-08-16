<script lang="ts">
  import { onMount } from "svelte";
  import {
    BrainCircuit,
    CircleDot,
    FileClock,
    Fingerprint,
    RefreshCw,
    X,
  } from "@lucide/svelte";
  import { getProviderCheckpoint, getWorkspaceMemoryOverview } from "$lib/api/providers";
  import { uiLocale } from "$lib/stores/locale";
  import { showToast } from "$lib/stores/toast";
  import {
    workspaceRootName,
    type ProviderCheckpoint,
    type WorkspaceMemoryOverview,
  } from "$lib/types";

  interface Props {
    workspaceId: string;
    workspacePath: string;
  }

  let { workspaceId, workspacePath }: Props = $props();
  let overview = $state<WorkspaceMemoryOverview | null>(null);
  let selected = $state<ProviderCheckpoint | null>(null);
  let loading = $state(false);
  let timer: number | undefined;
  let refreshGeneration = 0;
  const zh = $derived($uiLocale === "zh-CN");
  const workspaceName = $derived(workspaceRootName(workspacePath));
  const hasMemoryContent = $derived(Boolean(
    overview
    && (
      overview.currentFocus
      || overview.recentChanges.length
      || overview.openItems.length
      || overview.providerCheckpoints.length
    )
  ));

  function formatCapturedAt(value: string): string {
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) return value;
    return date.toLocaleString(zh ? "zh-CN" : "en-US", {
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      hour12: false,
    });
  }

  async function refresh() {
    if (!workspaceId || loading) return;
    const generation = ++refreshGeneration;
    loading = true;
    try {
      const next = await getWorkspaceMemoryOverview(workspaceId);
      if (generation === refreshGeneration) overview = next;
    } catch (error) {
      showToast(String(error), {
        title: zh ? "记忆状态读取失败" : "Failed to read memory state",
        kind: "error",
        duration: 8000,
      });
    } finally {
      if (generation === refreshGeneration) loading = false;
    }
  }

  async function inspect(checkpointId: string) {
    try {
      selected = await getProviderCheckpoint(workspaceId, checkpointId);
    } catch (error) {
      showToast(String(error), {
        title: zh ? "Checkpoint 读取失败" : "Failed to read checkpoint",
        kind: "error",
        duration: 8000,
      });
    }
  }

  onMount(() => {
    void refresh();
    timer = window.setInterval(() => void refresh(), 4000);
    return () => {
      refreshGeneration += 1;
      if (timer) window.clearInterval(timer);
    };
  });
</script>

<section class="page-scroll mn-memory-page-v2">
  <header class="page-header">
    <div class="mn-page-heading-row">
      <div>
        <p class="page-kicker">{zh ? "项目记忆" : "PROJECT MEMORY"}</p>
        <h2 class="page-title">{workspaceName}</h2>
        <p class="tx-project-path" title={workspacePath}>{workspacePath}</p>
      </div>
      <button type="button" class="mn-mini-action" disabled={loading} onclick={() => void refresh()}>
        <RefreshCw size={13} class={loading ? "animate-spin" : ""} /> {zh ? "刷新" : "Refresh"}
      </button>
    </div>
  </header>

  <div class="page-body mn-memory-workspace">
    {#if overview && hasMemoryContent}
      {#if overview.currentFocus || overview.recentChanges.length || overview.openItems.length}
        <div class="mn-memory-main-grid">
          {#if overview.currentFocus}
            <article class="mn-memory-focus-stage mn-memory-summary-card">
              <div class="mn-memory-stage-head">
                <div><CircleDot size={15} /><span>{zh ? "当前焦点" : "CURRENT FOCUS"}</span></div>
              </div>
              <div class="mn-memory-focus-body">
                <div class="mn-memory-focus-signal"><i></i><i></i><i></i></div>
                <p>{overview.currentFocus}</p>
              </div>
            </article>
          {/if}

          {#if overview.recentChanges.length}
            <article class="mn-memory-action-panel changes mn-memory-summary-card">
              <div class="mn-memory-stage-head">
                <div><FileClock size={15} /><span>{zh ? "最近变化" : "RECENT CHANGES"}</span></div>
                <b>{overview.recentChanges.length}</b>
              </div>
              <ol class="mn-memory-timeline">
                {#each overview.recentChanges as item}
                  <li><span class="mn-memory-timeline-dot"></span><p>{item}</p></li>
                {/each}
              </ol>
            </article>
          {/if}

          {#if overview.openItems.length}
            <article class="mn-memory-action-panel open mn-memory-summary-card">
              <div class="mn-memory-stage-head">
                <div><Fingerprint size={15} /><span>{zh ? "未完成项" : "OPEN ITEMS"}</span></div>
                <b>{overview.openItems.length}</b>
              </div>
              <ul class="mn-memory-open-list">
                {#each overview.openItems as item}
                  <li><i></i><span>{item}</span></li>
                {/each}
              </ul>
            </article>
          {/if}
        </div>
      {/if}

      {#if overview.providerCheckpoints.length}
        <section class="mn-memory-provenance-v2">
          <div class="mn-panel-cap">
            <div><span>PROVIDER</span><strong>{zh ? "最近 checkpoint" : "Recent checkpoints"}</strong></div>
            <b>{overview.providerCheckpoints.length}</b>
          </div>
          <div class="mn-checkpoint-rail">
            {#each overview.providerCheckpoints as checkpoint}
              <button
                type="button"
                class:selected={selected?.checkpointId === checkpoint.checkpointId}
                onclick={() => void inspect(checkpoint.checkpointId)}
              >
                <span class="mn-checkpoint-provider">{checkpoint.providerId}</span>
                <time>{formatCapturedAt(checkpoint.capturedAt)}</time>
              </button>
            {/each}
          </div>
        </section>
      {/if}

      {#if selected}
        <article class="mn-memory-inspector-v2">
          <div class="mn-panel-cap">
            <div><span>CHECKPOINT</span><strong>{selected.providerId} · {selected.checkpointId.slice(0, 12)}</strong></div>
            <button type="button" class="mn-icon-quiet" onclick={() => (selected = null)} aria-label={zh ? "关闭" : "Close"}><X size={14} /></button>
          </div>
          <div class="mn-checkpoint-meta-grid">
            <div><span>{zh ? "来源" : "SOURCE"}</span><strong>{selected.source}</strong></div>
            <div><span>THREAD</span><strong title={selected.providerSessionId}>{selected.providerSessionId.slice(0, 16)}</strong></div>
            <div><span>TURN</span><strong title={selected.providerTurnId ?? ""}>{selected.providerTurnId?.slice(0, 16) ?? "—"}</strong></div>
            <div><span>SHA</span><strong title={selected.contentSha256}>{selected.contentSha256.slice(0, 16)}</strong></div>
          </div>
          <details class="mn-checkpoint-json">
            <summary>{zh ? "查看原始 checkpoint 快照" : "View raw checkpoint snapshot"}</summary>
            <pre>{JSON.stringify(selected.turnSnapshot, null, 2)}</pre>
          </details>
        </article>
      {/if}
    {:else if overview}
      <div class="mn-memory-empty-state">
        <div class="mn-memory-empty-orbit"><BrainCircuit size={28} /></div>
        <strong>{zh ? "还没有项目记忆" : "No project memory yet"}</strong>
      </div>
    {:else}
      <div class="mn-memory-empty-state">
        <div class="mn-memory-empty-orbit"><BrainCircuit size={28} /></div>
        <strong>{zh ? "读取记忆…" : "Reading memory…"}</strong>
      </div>
    {/if}
  </div>
</section>
