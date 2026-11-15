<script lang="ts">
  import { Check, CircleDashed, CircleSlash, LoaderCircle, Play, RefreshCw, RotateCcw, Square, Trash2, TriangleAlert, X } from "lucide-svelte";
  import Dialog from "./Dialog.svelte";
  import NamePicks from "./NamePicks.svelte";
  import { commands, type GameStep, type NamesMode, type StepWorkDto } from "./bindings";
  import { finishedText, formatCount, gameSteps, leftLabel, orderedSteps, requestsLabel, stepHints, stepOutcomes, stepTitles, type StepOutcome } from "./game-run";
  import type { GameRun } from "./game-run.svelte";
  import { characterName, languageList, type Character } from "./glossary";
  import { activeRun } from "./run-status.svelte";

  /**
   * Окно «Выполнение»: шаги модели по всей игре одной очередью. Открывается сразу:
   * сколько осталось каждому шагу, считается только по кнопке. Запуск, отбор имён и итог живут в `game`.
   */
  let { projectId, game, onClose }: { projectId: string; game: GameRun; onClose: () => void } = $props();

  /** Сводка «сколько осталось». `null`: не считали или она устарела после запуска. */
  let work = $state<StepWorkDto[] | null>(null);
  let counting = $state(false);
  let error = $state("");
  let chosen = $state<GameStep[]>([...gameSteps]);
  let names = $state<NamesMode>("auto");
  let characters = $state<Character[]>([]);

  const toRun = $derived(orderedSteps(chosen));
  const requests = $derived(work?.filter((item) => toRun.includes(item.step)).reduce((sum, item) => sum + item.requests, 0) ?? null);
  /** Модель занята этой цепочкой или другим запуском. */
  const busy = $derived(activeRun.current !== null || game.active);
  const outcomes = $derived(game.result && !game.active ? stepOutcomes(game.requested, game.result) : null);

  /** Подсчёт идёт по всем сценам и репликам и занимает время, поэтому только по кнопке. */
  async function count() {
    counting = true;
    error = "";
    try {
      work = await commands.gameOverview(projectId);
    } catch (e) {
      error = String(e);
    } finally {
      counting = false;
    }
  }

  /** Цепочка кончилась или встала на отборе: прежняя сводка больше не верна. */
  let wasActive = false;
  $effect(() => {
    const active = game.active;
    if (wasActive && !active) work = null;
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
    chosen = on ? [...chosen, step] : chosen.filter((item) => item !== step);
  }

  /** Галочка «Все шаги»: отмечены все, ни одного или часть. */
  const allChosen = $derived(toRun.length === gameSteps.length);
  const someChosen = $derived(toRun.length > 0 && !allChosen);

  /** Отмечены не все: отмечает все. Отмечены все: снимает все. */
  function toggleAll() {
    chosen = allChosen ? [] : [...gameSteps];
  }

  function owner(id: number) {
    const character = characters.find((known) => known.id === id);
    return character ? characterName(character) : "Персонаж";
  }

  /** Подпись шага: что он делает, а после подсчёта сколько ему осталось. */
  function stepNote(step: GameStep) {
    const item = work?.find((known) => known.step === step);
    return item ? leftLabel(item) : stepHints[step];
  }

  function startTitle() {
    if (game.names) return "Сначала отбор имён";
    if (busy) return "Модель уже работает";
    if (toRun.length === 0) return "Отметьте хотя бы один шаг";
    return "Отмеченные шаги пойдут по порядку одной очередью. Шаг, которому нечего делать, пройдёт без запросов";
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
        chosen = [...gameSteps];
        work = null;
      }
    } finally {
      resetting = false;
    }
  }
</script>

<Dialog
  title="Выполнение"
  note="Отмеченные шаги идут по порядку одной очередью. Повторный запуск продолжает с того же места: сделанное пропускается"
  closeTitle="Закрыть. Запуск при этом продолжается"
  {onClose}
>
  {#snippet actions()}
    <!-- Сброс стоит в шапке иконкой, подальше от запуска, и спрашивает подтверждение. -->
    <button
      class="icon-button danger"
      aria-label="Сбросить всё"
      title={busy ? "Сбросить всё. Модель уже работает" : "Сбросить всё: вернуть проект к состоянию сразу после считывания игры"}
      disabled={busy || resetting}
      onclick={() => (confirmReset = true)}><RotateCcw size={14} /></button
    >
    <button
      class="icon-button"
      aria-label="Посчитать, сколько осталось"
      title="Посчитать, сколько осталось каждому шагу и сколько это запросов. Подсчёт идёт по всей игре и занимает время"
      disabled={counting || game.active}
      onclick={count}
    >
      <RefreshCw size={14} class={counting ? "spin" : ""} />
    </button>
  {/snippet}

  <!-- Запуск в подвале: шаги листаются, он на месте. -->
  {#snippet foot()}
    <span class="label">Имена персонажей</span>
    <div class="segmented" role="radiogroup" aria-label="Выбор написания имён">
      <button
        role="radio"
        aria-checked={names === "auto"}
        class:on={names === "auto"}
        disabled={busy}
        title="Написание, на котором перевод и проверка сошлись, пишется сразу. Спорные цепочка покажет в отборе"
        onclick={() => (names = "auto")}>Авто</button
      >
      <button
        role="radio"
        aria-checked={names === "manual"}
        class:on={names === "manual"}
        disabled={busy}
        title="Все написания идут в отбор: цепочка остановится на нём и продолжится после записи"
        onclick={() => (names = "manual")}>Ручной выбор</button
      >
    </div>
    <span class="faint grow">{requests !== null && toRun.length > 0 ? `${requestsLabel(requests)} по отмеченным шагам сейчас` : ""}</span>
    <button class="btn small primary" disabled={busy || toRun.length === 0 || game.names !== null} title={startTitle()} onclick={() => game.start(toRun, names, onError)}>
      {#if game.active}<LoaderCircle size={13} class="spin" />{:else}<Play size={13} />{/if}
      <span>Запустить · {toRun.length}</span>
    </button>
  {/snippet}

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

  {#if error}
    <div class="banner error" role="alert">
      <TriangleAlert size={14} />
      <span>{error}</span>
      <button class="icon-button" aria-label="Скрыть ошибку" title="Скрыть ошибку" onclick={() => (error = "")}><X size={14} /></button>
    </div>
  {/if}

  <label class="all" title={allChosen ? "Снять все шаги" : "Отметить все шаги"}>
    <input type="checkbox" checked={allChosen} indeterminate={someChosen} disabled={busy || game.names !== null} onchange={toggleAll} />
    <span>Все шаги</span>
    <span class="faint">отмечено {toRun.length} из {gameSteps.length}</span>
  </label>

  <ol class="steps">
    {#each gameSteps as step, index (step)}
      {@const item = work?.find((known) => known.step === step)}
      <li class:off={!chosen.includes(step)}>
        <input
          type="checkbox"
          checked={chosen.includes(step)}
          disabled={busy || game.names !== null}
          onchange={(event) => toggle(step, event.currentTarget.checked)}
          aria-label="Шаг «{stepTitles[step]}»"
        />
        <span class="number">{index + 1}</span>
        <div class="step">
          <strong>{stepTitles[step]}</strong>
          <span>{stepNote(step)}</span>
        </div>
        <span class="requests">{item && !item.empty ? requestsLabel(item.requests) : ""}</span>
      </li>
    {/each}
  </ol>

  {#if game.names}
    <div class="banner warn" role="status">
      <TriangleAlert size={14} />
      <span>
        Цепочка ждёт отбора имён: в режиме «Авто» сюда попадают написания, в которых перевод и проверка разошлись. Отмеченные запишутся
        в пустые поля персонажей, потом цепочка спросит формы этих персонажей и пойдёт дальше
      </span>
    </div>
    {#if game.missingRules.length > 0}
      <div class="banner warn" role="status">
        <TriangleAlert size={14} />
        <span>Нет правил для языков: {languageList(game.missingRules)}. Имена этих языков написаны без них</span>
      </div>
    {/if}
    <NamePicks items={game.names} {owner} />
    <div class="actions">
      <button class="btn small" disabled={game.writing} onclick={() => game.skipNames(onError)}>Отбросить и продолжить</button>
      <button class="btn small primary" disabled={game.writing} onclick={() => game.writeNames(onError)}>
        {#if game.writing}<LoaderCircle size={13} class="spin" />{:else}<Check size={13} />{/if}
        <span>Записать и продолжить · {game.chosen.length}</span>
      </button>
    </div>
  {/if}

  {#if outcomes && game.result}
    <div class="title result">
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
  {/if}
</Dialog>

<style>
  .title {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .title.result {
    flex: 0 0 auto;
    padding-top: 10px;
    border-top: 1px solid var(--divider);
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
    grid-template-columns: auto 16px 1fr auto;
    gap: 10px;
    align-items: center;
    padding: 6px 4px;
    border-bottom: 1px solid var(--divider-row);
  }

  /* Галочка стоит в той же колонке, что галочки шагов. */
  .all {
    display: flex;
    gap: 10px;
    align-items: center;
    margin-bottom: -10px;
    padding: 4px 4px 6px;
    border-bottom: 1px solid var(--divider);
    cursor: pointer;
  }

  .all span:first-of-type {
    color: var(--text);
    font-size: 12px;
    font-weight: 600;
  }

  .all .faint {
    margin-left: auto;
  }

  .steps li.off,
  .steps li.outcome.skipped {
    opacity: 0.55;
  }

  .steps li.outcome {
    grid-template-columns: 16px 1fr;
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

  .label {
    color: var(--text-muted);
    font-size: 11px;
  }

  .grow {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-align: right;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .actions {
    display: flex;
    gap: 8px;
    align-items: center;
    justify-content: flex-end;
  }
</style>
