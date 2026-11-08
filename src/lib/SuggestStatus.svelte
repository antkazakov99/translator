<script lang="ts" generics="T">
  import { Check, LoaderCircle, Square, X } from "lucide-svelte";
  import type { SuggestRun } from "./suggest-run.svelte";

  let { run, onError }: { run: SuggestRun<T>; onError: (message: string) => void } = $props();
</script>

{#if run.active}
  <div class="banner progress page-error" role="status">
    <LoaderCircle size={14} class="spin" />
    <span>{run.text}</span>
    <button class="btn small" disabled={run.stopping} onclick={() => run.stop(onError)}>
      <Square size={13} />
      <span>Остановить</span>
    </button>
  </div>
{:else if run.notice}
  <div class="banner page-error" role="status">
    <Check size={14} />
    <span>{run.notice}</span>
    <button class="icon-button" aria-label="Скрыть итог" title="Скрыть итог" onclick={() => (run.notice = "")}><X size={14} /></button>
  </div>
{/if}
