<script lang="ts">
  import CopyButton from "$lib/components/CopyButton.svelte";
  import { uiLocale } from "$lib/stores/locale";

  interface Props {
    value?: string;
    placeholder?: string;
    readonly?: boolean;
    disabled?: boolean;
    showCopy?: boolean;
    onRegenerate?: (() => void) | undefined;
    regenerating?: boolean;
    monospace?: boolean;
    size?: "sm" | "md";
  }

  let {
    value = $bindable(""),
    placeholder = "",
    readonly = false,
    disabled = false,
    showCopy = true,
    onRegenerate,
    regenerating = false,
    monospace = true,
    size = "md",
  }: Props = $props();

  const zh = $derived($uiLocale === "zh-CN");

  const isLoadingPlaceholder = $derived(value === "加载中…" || value === "Loading…");
  const fontClass = $derived(monospace ? "font-mono" : "");
  const textClass = $derived(size === "sm" ? "text-xs" : "text-sm");
</script>

<div class="flex gap-2">
  <div class="tx-secret-input min-w-0 flex-1">
    {#if readonly}
      <input
        type="text"
        class="tx-secret-input-field {fontClass} {textClass}"
        {value}
        {placeholder}
        readonly
        {disabled}
        autocomplete="off"
      />
    {:else}
      <input
        type="text"
        class="tx-secret-input-field {fontClass} {textClass}"
        bind:value
        {placeholder}
        {disabled}
        autocomplete="off"
      />
    {/if}
  </div>
  {#if showCopy && value && !isLoadingPlaceholder}
    <CopyButton {value} />
  {/if}
  {#if onRegenerate}
    <button
      type="button"
      class="shrink-0 rounded-md border border-[var(--color-border)] px-2.5 py-1 text-xs text-[var(--color-text-secondary)] transition-colors hover:bg-[var(--color-surface-hover)] disabled:opacity-50"
      disabled={regenerating || disabled}
      onclick={() => onRegenerate?.()}
    >
      {regenerating ? (zh ? "生成中…" : "Generating…") : (zh ? "重新生成" : "Regenerate")}
    </button>
  {/if}
</div>
