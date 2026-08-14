<script lang="ts">
  import { onMount } from "svelte";
  import { message } from "@tauri-apps/plugin-dialog";
  import {
    CheckCircle2,
    CircleAlert,
    Download,
    ExternalLink,
    RefreshCw,
    ShieldCheck,
  } from "@lucide/svelte";
  import { openUrl } from "$lib/api/app-info";
  import { RELEASES_LATEST_URL } from "$lib/app-links";
  import {
    checkForUpdates,
    installAvailableUpdate,
    updateState,
  } from "$lib/stores/update";

  function formatBytes(value: number): string {
    if (!Number.isFinite(value) || value <= 0) return "0 B";
    const units = ["B", "KB", "MB", "GB"];
    const index = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1);
    const amount = value / 1024 ** index;
    return `${amount >= 10 || index === 0 ? amount.toFixed(0) : amount.toFixed(1)} ${units[index]}`;
  }

  function progressPercent(): number | null {
    if (!$updateState.totalBytes || $updateState.totalBytes <= 0) return null;
    return Math.min(100, Math.round(($updateState.downloadedBytes / $updateState.totalBytes) * 100));
  }

  async function installUpdate() {
    try {
      await installAvailableUpdate();
    } catch (error) {
      await message(String(error), { title: "更新失败", kind: "error" });
    }
  }

  async function openReleases() {
    try {
      await openUrl(RELEASES_LATEST_URL);
    } catch (error) {
      await message(String(error), { title: "无法打开 Releases", kind: "error" });
    }
  }

  onMount(() => {
    if ($updateState.phase === "idle") {
      void checkForUpdates();
    }
  });
</script>

<section class="page-scroll">
  <header class="page-header">
    <p class="page-kicker">RELEASE CHANNEL</p>
    <h2 class="page-title">更新</h2>
    <p class="mt-2 max-w-2xl text-sm text-[var(--color-text-muted)]">
      Codex-Web 直接从 GitHub Releases 获取签名更新包。发现新版后可在这里完成下载、安装与重启。
    </p>
  </header>

  <div class="page-body flex max-w-3xl flex-col gap-5">
    <div class="tx-card overflow-hidden">
      <div class="flex flex-wrap items-start justify-between gap-4 p-5">
        <div>
          <p class="text-xs font-medium uppercase tracking-[0.16em] text-[var(--color-text-muted)]">
            Current build
          </p>
          <div class="mt-2 flex items-baseline gap-3">
            <strong class="font-mono text-2xl font-semibold">v{$updateState.currentVersion}</strong>
            {#if $updateState.phase === "up-to-date"}
              <span class="inline-flex items-center gap-1 text-xs text-emerald-400">
                <CheckCircle2 size={13} /> 已是最新
              </span>
            {:else if $updateState.phase === "available" && $updateState.latestVersion}
              <span class="text-xs text-[var(--color-accent)]">
                → v{$updateState.latestVersion}
              </span>
            {/if}
          </div>
        </div>

        <button
          type="button"
          class="tx-btn-ghost inline-flex items-center gap-2"
          disabled={$updateState.phase === "checking" || $updateState.phase === "downloading" || $updateState.phase === "installing"}
          onclick={() => void checkForUpdates()}
        >
          <RefreshCw
            size={14}
            class={$updateState.phase === "checking" ? "animate-spin" : ""}
          />
          {$updateState.phase === "checking" ? "检查中…" : "重新检查"}
        </button>
      </div>

      {#if $updateState.phase === "available"}
        <div class="border-t border-[var(--color-border)] bg-[color-mix(in_srgb,var(--color-accent)_4%,transparent)] p-5">
          <div class="flex flex-wrap items-center justify-between gap-4">
            <div>
              <p class="text-sm font-semibold">发现新版本 v{$updateState.latestVersion}</p>
              <p class="mt-1 text-xs text-[var(--color-text-muted)]">
                更新包会先通过应用内置公钥验证签名，再执行安装。
              </p>
            </div>
            <button
              type="button"
              class="tx-btn-primary inline-flex items-center gap-2"
              onclick={() => void installUpdate()}
            >
              <Download size={15} />
              立即更新
            </button>
          </div>
        </div>
      {:else if $updateState.phase === "downloading" || $updateState.phase === "installing"}
        <div class="border-t border-[var(--color-border)] p-5">
          <div class="flex items-center justify-between gap-4 text-sm">
            <span class="font-medium">
              {$updateState.phase === "installing" ? "正在安装，随后自动重启…" : "正在下载更新…"}
            </span>
            {#if progressPercent() !== null}
              <span class="font-mono text-xs text-[var(--color-text-muted)]">{progressPercent()}%</span>
            {/if}
          </div>
          <div class="mt-3 h-1.5 overflow-hidden rounded-full bg-[var(--color-bg)]">
            <div
              class="h-full rounded-full bg-[var(--color-accent)] shadow-[0_0_14px_var(--color-accent)] transition-[width] duration-200"
              style={`width:${progressPercent() ?? 18}%`}
            ></div>
          </div>
          <p class="mt-2 font-mono text-[11px] text-[var(--color-text-muted)]">
            {formatBytes($updateState.downloadedBytes)}
            {#if $updateState.totalBytes}
              / {formatBytes($updateState.totalBytes)}
            {/if}
          </p>
        </div>
      {:else if $updateState.phase === "error"}
        <div class="flex items-start gap-3 border-t border-[var(--color-border)] p-5">
          <CircleAlert class="mt-0.5 shrink-0 text-amber-400" size={17} />
          <div class="min-w-0">
            <p class="text-sm font-medium">暂时无法检查更新</p>
            <p class="mt-1 break-words text-xs text-[var(--color-text-muted)]">{$updateState.error}</p>
          </div>
        </div>
      {/if}
    </div>

    {#if $updateState.phase === "available" && $updateState.notes}
      <div class="tx-card p-5">
        <p class="text-xs font-medium uppercase tracking-[0.16em] text-[var(--color-text-muted)]">Release notes</p>
        <pre class="mt-3 whitespace-pre-wrap break-words font-sans text-sm leading-6 text-[var(--color-text)]">{$updateState.notes}</pre>
      </div>
    {/if}

    <div class="tx-card flex flex-wrap items-center justify-between gap-4 p-4">
      <div class="flex min-w-0 items-start gap-3">
        <ShieldCheck class="mt-0.5 shrink-0 text-[var(--color-accent)]" size={17} />
        <div>
          <p class="text-sm font-medium">GitHub Release + 签名 OTA</p>
          <p class="mt-1 text-xs text-[var(--color-text-muted)]">
            如果自动更新不可用，仍可直接打开 Releases 下载完整安装包。
          </p>
        </div>
      </div>
      <button type="button" class="tx-btn-ghost inline-flex items-center gap-2" onclick={() => void openReleases()}>
        <ExternalLink size={14} />
        打开 Releases
      </button>
    </div>
  </div>
</section>
