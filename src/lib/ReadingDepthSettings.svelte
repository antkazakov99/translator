<script lang="ts">
  import { onMount } from "svelte";
  import { LoaderCircle, RotateCcw } from "lucide-svelte";
  import { commands } from "./bindings";
  import { DEFAULT_READING_DEPTH, normalizeReadingDepth, type ReadingDepth } from "./reading-depth";
  import { depthChanges } from "./reading-depth.svelte";

  /** Глубина чтения приложения: сколько частей модель читает, когда описывает сцены, персонажей и термины. */
  const id = $props.id();

  let saved = $state<ReadingDepth | null>(null);
  /** Поля как текст: пустой максимум значит «без предела». */
  let min = $state("");
  let max = $state("");
  let percent = $state("");
  let error = $state("");
  let saving = $state(false);

  const isDefault = $derived(
    saved !== null && saved.min === DEFAULT_READING_DEPTH.min && saved.max === DEFAULT_READING_DEPTH.max && saved.percent === DEFAULT_READING_DEPTH.percent,
  );

  onMount(async () => {
    try {
      show(await commands.readingDepth());
    } catch (e) {
      error = String(e);
    }
  });

  function show(depth: { min: number; max?: number | null; percent: number }) {
    saved = { min: depth.min, max: depth.max ?? null, percent: depth.percent };
    min = String(saved.min);
    max = saved.max === null ? "" : String(saved.max);
    percent = String(saved.percent);
  }

  /** Целое из поля. `null`: поле пустое. `NaN`: в поле не число. */
  function number(text: string): number | null {
    const trimmed = text.trim();
    if (!trimmed) return null;
    return /^\d+$/.test(trimmed) ? Number(trimmed) : NaN;
  }

  async function save(next?: ReadingDepth) {
    const minValue = number(min);
    const maxValue = number(max);
    const percentValue = number(percent);
    if (!next) {
      if (minValue === null || Number.isNaN(minValue) || minValue < 1) return void (error = "Минимум частей это целое число от 1");
      if (maxValue !== null && (Number.isNaN(maxValue) || maxValue < minValue))
        return void (error = "Максимум частей это целое число не меньше минимума или пустое поле");
      if (percentValue === null || Number.isNaN(percentValue) || percentValue < 1 || percentValue > 100)
        return void (error = "Процент частей это целое число от 1 до 100");
    }
    const value = next ?? normalizeReadingDepth({ min: minValue ?? 1, max: maxValue, percent: percentValue ?? 100 });
    saving = true;
    try {
      await commands.saveReadingDepth(value);
      show(value);
      error = "";
      depthChanges.count++;
    } catch (e) {
      error = String(e);
    } finally {
      saving = false;
    }
  }
</script>

<section class="depth" aria-labelledby="{id}-title">
  <div class="head">
    <div class="title">
      <strong id="{id}-title">Глубина чтения</strong>
      <span>Сколько частей модель читает, когда описывает сцены, их персонажей и термины. Общая для всех проектов</span>
    </div>
    {#if saving}<LoaderCircle size={13} class="spin" />{/if}
    <button class="btn small" disabled={saved === null || isDefault || saving} title="От 5 до 50 частей, 60%" onclick={() => save({ ...DEFAULT_READING_DEPTH })}>
      <RotateCcw size={13} />
      <span>По умолчанию</span>
    </button>
  </div>

  {#if saved}
    <div class="fields">
      <label class="field">
        Минимум частей
        <input class="input" inputmode="numeric" bind:value={min} onchange={() => save()} disabled={saving} />
      </label>
      <label class="field">
        Максимум частей
        <input class="input" inputmode="numeric" bind:value={max} onchange={() => save()} disabled={saving} placeholder="Без предела" />
      </label>
      <label class="field">
        Процент частей
        <input class="input" inputmode="numeric" bind:value={percent} onchange={() => save()} disabled={saving} />
      </label>
    </div>
    <p class="note">
      Число частей по проценту округляется вверх и держится между минимумом и максимумом. Пустой максимум снимает предел. У кого частей меньше
      минимума, тот читается целиком. Прочитанные части остаются и не читаются заново
    </p>
  {:else if !error}
    <p class="note"><LoaderCircle size={13} class="spin" /> Загрузка</p>
  {/if}
  {#if error}<p class="error-text">{error}</p>{/if}
</section>

<style>
  .depth {
    display: flex;
    flex-direction: column;
    gap: 8px;
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

  .fields {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(140px, 1fr));
    gap: 10px 16px;
  }

  .note {
    display: flex;
    gap: 6px;
    align-items: center;
    margin: 0;
  }

  .error-text {
    margin: 0;
  }
</style>
