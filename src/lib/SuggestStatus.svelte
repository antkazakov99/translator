<script lang="ts" generics="T">
  import { Check, X } from "lucide-svelte";
  import type { SuggestRun, WriteRun } from "./suggest-run.svelte";

  let { run, onError }: { run: SuggestRun<T> | WriteRun; onError: (message: string) => void } = $props();
</script>

{#if !run.active && run.notice}
  <div class="banner page-error" role="status">
    <Check size={14} />
    <span>{run.notice}</span>
    <button class="icon-button" aria-label="Скрыть итог" title="Скрыть итог" onclick={() => (run.notice = "")}><X size={14} /></button>
  </div>
{/if}
