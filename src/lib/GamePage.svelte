<script lang="ts">
  import { onMount } from "svelte";
  import { Check, CircleDashed, CircleSlash, LoaderCircle, Play, RefreshCw, RotateCcw, Square, Trash2, TriangleAlert } from "lucide-svelte";
  import NamePicks from "./NamePicks.svelte";
  import { commands, type GameStep, type NamesMode, type StepWorkDto } from "./bindings";
  import { availableSteps, defaultSteps, fedBy, finishedText, formatCount, leftLabel, requestsLabel, stepOutcomes, stepTitles, stepsToRun, type StepOutcome } from "./game-run";
  import type { GameRun } from "./game-run.svelte";
  import { characterName, languageList, type Character } from "./glossary";
  import { activeRun } from "./run-status.svelte";

  /** Вид «Вся игра»: шаги модели по всей игре одной очередью. Запуск, отбор имён и итог живут в `game`. */
  let { projectId, game }: { projectId: string; game: GameRun } = $props();

  let work = $state<StepWorkDto[] | null>(null);
  let loading = $state(false);
  let error = $state("");
  /** Отмеченные шаги. `null`: пользователь ещё не трогал, отмечено всё, где есть что делать. */
  let chosen = $state<GameStep[] | null>(null);
  let names = $state<NamesMode>("auto");
  let characters = $state<Character[]>([]);

  const picked = $derived(chosen ?? (work ? defaultSteps(work) : []));
  const toRun = $derived(work ? stepsToRun(picked, work) : []);
  /** Отмечены шаги, работа которых появится только по ходу цепочки: их запросы пока не сосчитать. */
  const later = $derived(work?.some((item) => item.empty && toRun.includes(item.step)) ?? false);
  const available = $derived(work ? availableSteps(picked, work) : new Set<GameStep>());
  const requests = $derived(work?.filter((item) => toRun.includes(item.step)).reduce((sum, item) => sum + item.requests, 0) ?? 0);
  /** Модель занята этой цепочкой или другим запуском. */
  const busy = $derived(activeRun.current !== null || game.active);
  const outcomes = $derived(game.result && !game.active ? stepOutcomes(game.requested, game.result) : null);

  async function load() {
    loading = true;
    error = "";
    try {
      work = await commands.gameOverview(projectId);
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  onMount(load);

  /** Цепочка кончилась или встала на отборе: сводка пересчитывается. */
  let wasActive = false;
  $effect(() => {
    const active = game.active;
    if (wasActive && !active) load();
    wasActive = active;
  });

  /** Отбор имён показывает владельцев: персонажи берутся из глоссария. */
  $effect(() => {
    if (!game.names) return;
    commands
      .glossary(projectId)
      .then((data) => (characters = data.characters))
      .catch((e) => (error = String(e)));
  });

  function toggle(step: GameStep, on: boolean) {
    const next = new Set(picked);
    if (on) next.add(step);
    else next.delete(step);
    chosen = [...next];
  }

  function owner(id: number) {
    const character = characters.find((known) => known.id === id);
    return character ? characterName(character) : "Персонаж";
  }

  function startTitle() {
    if (game.names) return "Сначала отбор имён";
    if (busy) return "Модель уже работает";
    if (toRun.length === 0) return "Отметьте шаги, где есть что делать";
    return "Отмеченные шаги пойдут по порядку одной очередью";
  }

  function outcomeText(outcome: StepOutcome) {
    switch (outcome.kind) {
      case "done":
        return `Сделано · записано: ${formatCount(outcome.written)}`;
      case "error":
        return "Ошибка";
      case "stopped":
        return "Остановлено";
      case "waiting":
        return game.names ? "Ждёт отбора имён" : "Не дошла очередь";
      case "skipped":
        return "Пропущен";
    }
  }

  const onError = (message: string) => (error = message);

  /** Подтверждение сброса всего прогресса. */
  let confirmReset = $state(false);
  let resetting = $state(false);

  async function reset() {
    resetting = true;
    error = "";
    try {
      if (await game.reset(onError)) {
        confirmReset = false;
        chosen = null;
        await load();
      }
    } finally {
      resetting = false;
    }
  }
</script>

<div class="game-page">
  <section class="card">
    <div class="head">
      <div class="title">
        <strong>Вся игра</strong>
        <span>Отмеченные шаги идут по порядку одной очередью. Повторный запуск продолжает с того же места: сделанное пропускается</span>
      </div>
      <button class="icon-button" aria-label="Обновить сводку" title="Пересчитать, сколько осталось" disabled={loading} onclick={load}>
        <RefreshCw size={14} class={loading ? "spin" : ""} />
      </button>
    </div>

    {#if work}
      <ol class="steps">
        {#each work as item, index (item.step)}
          {@const ready = available.has(item.step)}
          {@const feeder = work ? fedBy(item.step, picked, work) : null}
          <li class:off={!ready}>
            <input
              type="checkbox"
              checked={ready && picked.includes(item.step)}
              disabled={!ready || busy || game.names !== null}
              onchange={(event) => toggle(item.step, event.currentTarget.checked)}
              aria-label="Шаг «{item.title}»"
            />
            <span class="number">{index + 1}</span>
            <div class="step">
              <strong>{item.title}</strong>
              <span>{feeder ? `Сейчас делать нечего: работа появится после шага «${stepTitles[feeder]}»` : leftLabel(item)}</span>
            </div>
            <span class="requests">{item.empty ? "" : requestsLabel(item.requests)}</span>
          </li>
        {/each}
      </ol>

      <div class="options">
        <span class="label">Имена персонажей</span>
        <div class="segmented" role="radiogroup" aria-label="Выбор написания имён">
          <button
            role="radio"
            aria-checked={names === "auto"}
            class:on={names === "auto"}
            disabled={busy}
            title="Берётся первый вариант модели: самое устоявшееся написание"
            onclick={() => (names = "auto")}>Авто</button
          >
          <button
            role="radio"
            aria-checked={names === "manual"}
            class:on={names === "manual"}
            disabled={busy}
            title="Цепочка остановится на отборе имён и продолжится после записи"
            onclick={() => (names = "manual")}>Ручной выбор</button
          >
        </div>
      </div>

      <div class="actions">
        <button class="btn small primary" disabled={busy || toRun.length === 0 || game.names !== null} title={startTitle()} onclick={() => game.start(toRun, names, onError)}>
          {#if game.active}<LoaderCircle size={13} class="spin" />{:else}<Play size={13} />{/if}
          <span>Запустить · {toRun.length}</span>
        </button>
        {#if toRun.length > 0}<span class="faint">{requestsLabel(requests)} всего{later ? " сейчас: шаги, где работа появится по ходу, добавят ещё" : ""}</span>{/if}
        <button class="btn small danger reset" disabled={busy || resetting} title={busy ? "Модель уже работает" : "Вернуть проект к состоянию сразу после считывания игры"} onclick={() => (confirmReset = true)}>
          <RotateCcw size={13} />
          <span>Сбросить всё</span>
        </button>
      </div>
      {#if confirmReset}
        <div class="banner error" role="alert">
          <TriangleAlert size={14} />
          <span>Удалятся переводы реплик с историей, описания сцен, заметки модели, весь глоссарий (персонажи, термины, подписи, токены) и запуски. Снимок игры и настройки останутся. Отменить нельзя</span>
          <button class="btn small" disabled={resetting} onclick={() => (confirmReset = false)}>Отмена</button>
          <button class="btn small danger" disabled={resetting || busy} onclick={reset}>
            {#if resetting}<LoaderCircle size={13} class="spin" />{:else}<Trash2 size={13} />{/if}
            <span>Удалить безвозвратно</span>
          </button>
        </div>
      {/if}
    {:else if loading}
      <div class="faint loading"><LoaderCircle size={13} class="spin" /> Считаю, сколько осталось</div>
    {/if}

    {#if error}
      <div class="banner error" role="alert">
        <TriangleAlert size={14} />
        <span>{error}</span>
      </div>
    {/if}
  </section>

  {#if game.names}
    <section class="card">
      <div class="banner warn" role="status">
        <TriangleAlert size={14} />
        <span>Цепочка ждёт отбора имён. Отмеченные написания запишутся в пустые поля персонажей, потом пойдут следующие шаги</span>
      </div>
      {#if game.missingRules.length > 0}
        <div class="banner warn" role="status">
          <TriangleAlert size={14} />
          <span>Нет правил для языков: {languageList(game.missingRules)}. Имена этих языков написаны без них</span>
        </div>
      {/if}
      <NamePicks items={game.names} {owner} />
      <div class="actions end">
        <button class="btn small" disabled={game.writing} onclick={() => game.skipNames(onError)}>Отбросить и продолжить</button>
        <button class="btn small primary" disabled={game.writing} onclick={() => game.writeNames(onError)}>
          {#if game.writing}<LoaderCircle size={13} class="spin" />{:else}<Check size={13} />{/if}
          <span>Записать и продолжить · {game.chosen.length}</span>
        </button>
      </div>
    </section>
  {/if}

  {#if outcomes && game.result}
    <section class="card">
      <div class="title">
        <strong>Последний запуск</strong>
        <span>{finishedText(game.result.finished, game.result.stopped)}</span>
      </div>
      <ol class="steps">
        {#each Object.entries(outcomes) as [step, outcome] (step)}
          <li class="outcome {outcome.kind}">
            <span class="mark">
              {#if outcome.kind === "done"}<Check size={13} />{:else if outcome.kind === "error"}<TriangleAlert size={13} />{:else if outcome.kind === "stopped"}<Square size={12} />{:else if outcome.kind === "waiting"}<CircleDashed size={13} />{:else}<CircleSlash size={13} />{/if}
            </span>
            <div class="step">
              <strong>{stepTitles[step as GameStep]}</strong>
              <span>{outcomeText(outcome)}</span>
              {#if outcome.kind === "error"}<span class="error-text">{outcome.message}</span>{/if}
            </div>
          </li>
        {/each}
      </ol>
    </section>
  {/if}
</div>

<style>
  .game-page {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 12px;
    width: 100%;
    max-width: 860px;
    margin: 0 auto;
    padding: 16px;
    overflow-y: auto;
  }

  .card {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .head {
    display: flex;
    gap: 6px;
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
  .faint {
    color: var(--text-faint);
    font-size: 11px;
  }

  .steps {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .steps li {
    display: grid;
    grid-template-columns: auto 18px 1fr auto;
    gap: 10px;
    align-items: center;
    padding: 6px 4px;
    border-bottom: 1px solid var(--divider-row);
  }

  .steps li.off,
  .steps li.outcome.skipped {
    opacity: 0.55;
  }

  .steps li.outcome {
    grid-template-columns: 18px 1fr;
  }

  .mark {
    display: inline-flex;
    color: var(--text-faint);
  }

  .outcome.done .mark {
    color: var(--success);
  }

  .outcome.error .mark {
    color: var(--danger);
  }

  .outcome.stopped .mark,
  .outcome.waiting .mark {
    color: var(--warning);
  }

  .number {
    color: var(--text-faint);
    font-size: 11px;
    text-align: right;
  }

  .step {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .step strong {
    color: var(--text);
    font-size: 12px;
    font-weight: 600;
  }

  .step span {
    color: var(--text-muted);
    font-size: 11px;
  }

  .requests {
    color: var(--text-faint);
    font-size: 11px;
    white-space: nowrap;
  }

  .options {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    align-items: center;
  }

  .label {
    color: var(--text-muted);
    font-size: 12px;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
  }

  /* Сброс справа, подальше от запуска. */
  .reset {
    margin-left: auto;
  }

  .actions.end {
    justify-content: flex-end;
  }

  .loading {
    display: flex;
    gap: 6px;
    align-items: center;
  }
</style>
