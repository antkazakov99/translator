<script lang="ts">
  import { LoaderCircle, OctagonX, Square, TriangleAlert } from "lucide-svelte";
  import { commands } from "./bindings";
  import { activeRun } from "./run-status.svelte";

  /** Ошибка остановки: запуск идёт дальше, полоса говорит об этом сама. */
  let error = $state("");
  /** Обрыв отправлен: ответ на идущий запрос уже не ждётся. */
  let aborting = $state(false);
  const run = $derived(activeRun.current);

  $effect(() => {
    if (!run) {
      error = "";
      aborting = false;
    }
  });

  /** Rust держит один запуск во всём приложении, поэтому обрыв не знает источника запуска. */
  async function abort() {
    aborting = true;
    try {
      await commands.abortLlmRun();
    } catch (cause) {
      aborting = false;
      error = String(cause);
    }
  }
</script>

{#if run}
  <div class="banner progress run-bar" role="status" aria-live="polite">
    <LoaderCircle size={14} class="spin" />
    <span>{run.text}</span>
    {#if error}
      <em class="run-error"><TriangleAlert size={12} /> {error}</em>
    {/if}
    <button
      class="btn small"
      disabled={run.stopping || aborting}
      title="Остановить после текущего запроса"
      onclick={() => run?.stop((message) => (error = message))}
    >
      <Square size={13} />
      <b>{run.stopping || aborting ? "Останавливается" : "Остановить"}</b>
    </button>
    <button class="btn small" disabled={aborting} title="Остановить сразу: ответ на текущий запрос теряется" onclick={abort}>
      <OctagonX size={13} />
      <b>Оборвать</b>
    </button>
  </div>
{/if}

<style>
  /* Полоса на всю ширину окна под шапкой, а не карточка внутри раздела. */
  .run-bar {
    padding: 7px 16px;
    border-width: 0 0 1px;
    border-radius: 0;
    font-size: 12px;
  }

  .run-error {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--danger);
    font-style: normal;
  }

  .run-bar b {
    font-weight: inherit;
  }
</style>
