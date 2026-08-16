<script lang="ts">
  import { onDestroy } from "svelte";
  import { uiLocale } from "$lib/stores/locale";

  interface Props {
    value: string;
    label?: string;
    onCopy?: () => void;
  }

  let { value, label, onCopy }: Props = $props();
  let copied = $state(false);
  let resetTimer: ReturnType<typeof setTimeout> | undefined;
  const zh = $derived($uiLocale === "zh-CN");
  const visibleLabel = $derived(label ?? (zh ? "复制" : "Copy"));

  onDestroy(() => {
    if (resetTimer !== undefined) clearTimeout(resetTimer);
  });

  async function copy() {
    await navigator.clipboard.writeText(value);
    copied = true;
    onCopy?.();
    if (resetTimer !== undefined) clearTimeout(resetTimer);
    resetTimer = setTimeout(() => {
      copied = false;
      resetTimer = undefined;
    }, 1500);
  }
</script>

<button
  type="button"
  class="tx-btn-ghost shrink-0 px-2.5 py-1 text-xs"
  onclick={copy}
>
  {copied ? (zh ? "已复制" : "Copied") : visibleLabel}
</button>
