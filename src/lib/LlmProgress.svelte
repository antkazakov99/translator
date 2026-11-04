<script lang="ts">
  import { onMount } from "svelte";
  import { llmRun } from "./llm-run.svelte";

  let now = $state(Date.now());
  let progress = $derived(llmRun.progress);
  const phases = { analysis: "Анализ происхождения", translation: "Перевод", validation: "Проверка перевода", rules: "Генерация правил", completed: "Пакет обработан" };
  const duration = (started: number) => {
    const seconds = Math.max(0, Math.floor((now - started) / 1000));
    return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
  };
  onMount(() => {
    const timer = setInterval(() => { now = Date.now(); }, 1000);
    return () => clearInterval(timer);
  });
</script>

<span class="request-progress">
  <span aria-live="polite">
    {#if progress}
      {phases[progress.phase]} · {progress.phase === "rules" ? "Запрос" : "Пакет"} {progress.batch} из {progress.total_batches}
      {#if progress.attempt > 1} · повторная попытка {progress.attempt}{/if}
      · обработано {progress.completed} из {progress.total}
      · {progress.phase === "completed" ? "осталось пакетов" : "после текущего"}: {Math.max(0, progress.total_batches - progress.batch)}
    {/if}
  </span>
  {#if progress?.names?.length && progress.phase !== "completed"}<span title={progress.names.join(", ")}>Сейчас: {progress.names.slice(0, 3).join(", ")}{progress.names.length > 3 ? ` и ещё ${progress.names.length - 3}` : ""}</span>{/if}
  <span>Прошло {duration(llmRun.startedAt)}{progress && progress.phase !== "completed" ? ` · текущий запрос ${duration(llmRun.requestStartedAt)}` : ""}</span>
  {#if progress && progress.total > 0}<progress value={progress.completed} max={progress.total} aria-label="Обработано записей"></progress>{/if}
</span>

<style>
  .request-progress { display: inline-flex; flex-wrap: wrap; gap: 4px 12px; font-size: 11px; color: #94a3b8; line-height: 1.5; }
  progress { width: 120px; height: 7px; align-self: center; accent-color: #818cf8; }
</style>
