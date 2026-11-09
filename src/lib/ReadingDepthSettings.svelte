<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { LoaderCircle, RotateCcw, X } from "lucide-svelte";
  import { commands } from "./bindings";
  import { DEFAULT_READING_DEPTH, normalizeReadingDepth, type ReadingDepth } from "./reading-depth";
  import { depthChanges } from "./reading-depth.svelte";
  import RequestSlider from "./RequestSlider.svelte";

  /** Глубина чтения проекта: сколько частей модель читает в групповом описании и перед переводом сцены. */
  let { projectId, onClose }: { projectId: string; onClose: () => void } = $props();
  const id = $props.id();

  let value = $state<ReadingDepth | null>(null);
  let error = $state("");
  let saving = $state(false);
  let timer: ReturnType<typeof setTimeout> | undefined;

  const partsLimit = $derived(Math.max(50, value?.max ?? 0));
  const isDefault = $derived(
    value !== null && value.min === DEFAULT_READING_DEPTH.min && value.max === DEFAULT_READING_DEPTH.max && value.percent === DEFAULT_READING_DEPTH.percent,
  );

  onMount(async () => {
    try {
      value = await commands.readingDepth(projectId);
    } catch (e) {
      error = String(e);
    }
  });

  onDestroy(() => {
    if (timer) {
      clearTimeout(timer);
      void save();
    }
  });

  function change(field: keyof ReadingDepth, amount: number) {
    if (!value) return;
    const next = { ...value, [field]: amount };
    // Максимум ниже минимума тянет минимум за собой, а не упирается в него.
    if (field === "max") next.min = Math.min(next.min, Math.max(1, Math.floor(next.max)));
    value = normalizeReadingDepth(next);
    schedule();
  }

  function reset() {
    value = { ...DEFAULT_READING_DEPTH };
    schedule();
  }

  /** Бегунок шлёт значения на каждом шаге: в базу уходит последнее. */
  function schedule() {
    clearTimeout(timer);
    timer = setTimeout(() => {
      timer = undefined;
      void save();
    }, 400);
  }

  async function save() {
    if (!value) return;
    saving = true;
    try {
      await commands.saveReadingDepth(projectId, value);
      error = "";
      depthChanges.count++;
    } catch (e) {
      error = String(e);
    } finally {
      saving = false;
    }
  }
</script>

<div class="card depth">
  <div class="head">
    <div class="title">
      <strong>Глубина чтения</strong>
      <span>
        Сколько частей читает модель, когда описывает группу сцен, персонажей или терминов и когда перевод читает сцену без описания. Описание
        одной сцены из её карточки читает её целиком
      </span>
    </div>
    {#if saving}<LoaderCircle size={13} class="spin" />{/if}
    <button class="btn small" disabled={!value || isDefault} title="От 5 до 50 частей, 30%" onclick={reset}>
      <RotateCcw size={13} />
      <span>По умолчанию</span>
    </button>
    <button class="icon-button" aria-label="Закрыть глубину чтения" title="Закрыть" onclick={onClose}><X size={14} /></button>
  </div>

  {#if error}<p class="error-text">{error}</p>{/if}

  {#if value}
    <RequestSlider
      id={`${id}-percent`}
      label="Процент частей"
      status={`${value.percent}%`}
      max={100}
      value={value.percent}
      onchange={(amount) => change("percent", amount)}
    >
      {#snippet ends()}<span>1%</span><span>все части · 100%</span>{/snippet}
    </RequestSlider>
    <div class="bounds">
      <RequestSlider
        id={`${id}-min`}
        label="Минимум частей"
        status={String(value.min)}
        max={partsLimit}
        value={value.min}
        onchange={(amount) => change("min", amount)}
      >
        {#snippet ends()}<span>1</span><span>{partsLimit}</span>{/snippet}
      </RequestSlider>
      <RequestSlider
        id={`${id}-max`}
        label="Максимум частей"
        status={String(value.max)}
        max={partsLimit}
        value={value.max}
        onchange={(amount) => change("max", amount)}
      >
        {#snippet ends()}<span>1</span><span>{partsLimit}</span>{/snippet}
      </RequestSlider>
    </div>
    <p class="note">
      Число частей по проценту округляется вверх и держится между минимумом и максимумом. У кого частей меньше минимума, тот читается целиком.
      Прочитанные части остаются и не читаются заново
    </p>
  {:else if !error}
    <p class="note"><LoaderCircle size={13} class="spin" /> Загрузка</p>
  {/if}
</div>

<style>
  .depth {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .head {
    display: flex;
    gap: 8px;
    align-items: flex-start;
  }

  .title {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 2px;
  }

  .title strong {
    color: var(--text-strong);
    font-size: 13px;
    font-weight: 600;
  }

  .title span,
  .note {
    color: var(--text-faint);
    font-size: 11px;
  }

  .bounds {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
    gap: 0 20px;
  }

  .note {
    display: flex;
    gap: 6px;
    align-items: center;
    margin: 8px 0 0;
  }

  .error-text {
    margin: 0;
  }
</style>
