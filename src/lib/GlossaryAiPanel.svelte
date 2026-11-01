<script lang="ts">
  import { onDestroy, type Snippet } from "svelte";
  import { Channel, invoke } from "@tauri-apps/api/core";
  import { AlertCircle, Check, ChevronDown, ChevronRight, Loader2, Sparkles, Square } from "lucide-svelte";
  import EntryPicker from "./EntryPicker.svelte";
  import LlmChoice from "./LlmChoice.svelte";
  import { characterTitle, type NameForm } from "./name-forms";
  import { beginLlmRun, endLlmRun, llmRun } from "./llm-run.svelte";
  import { LLM_ACTIONS, outsideNote, targetFor, type LlmAction, type LlmActionSettings, type LlmSettings } from "./llm-settings";

  interface PanelCharacter {
    alias: string;
    original_name: string;
    translated_name: string;
    merged: { alias: string; original_name: string }[];
    card_name: string;
  }

  interface PanelTerm {
    id: string;
    source_term: string;
    translated_term: string;
    category: string;
    notes: string;
    forms: NameForm[];
  }

  interface PanelToken {
    name: string;
    lines: number;
    value: string;
    note: string;
  }

  interface AutofillResult<T> {
    items: T[];
    failed_batches: number;
  }

  interface BatchProgress<T> {
    batch: number;
    total_batches: number;
    failed_batches: number;
    items: T[];
  }

  interface NameSuggestion {
    original: string;
    translated: string;
    origin: string;
  }

  interface TokenSuggestion {
    name: string;
    value: string;
    note: string;
  }

  type GlossaryTab = "characters" | "terms" | "tokens";

  let {
    tab,
    gameDir,
    characters,
    terms,
    tokens,
    hints,
    settings,
    descriptionsOpen = $bindable(false),
    descriptions,
    onselect,
    onnames,
    onterms,
    ontokens
  }: {
    tab: GlossaryTab;
    gameDir: string;
    characters: PanelCharacter[];
    terms: PanelTerm[];
    tokens: PanelToken[];
    hints: object;
    settings: LlmSettings;
    descriptionsOpen?: boolean;
    descriptions: Snippet;
    onselect: (action: LlmAction, choice: LlmActionSettings) => void;
    onnames: (rows: { alias: string; translated: string }[]) => void;
    onterms: (rows: { id: string; translated: string; category: string; notes: string }[]) => void;
    ontokens: (rows: { name: string; value: string; note: string }[]) => void;
  } = $props();

  const SECTIONS: Record<GlossaryTab, LlmAction[]> = {
    characters: ["descriptions", "names"],
    terms: ["terms"],
    tokens: ["tokens"]
  };

  let panelOpen = $state(false);
  let sectionByTab = $state<Record<GlossaryTab, LlmAction>>({ characters: "descriptions", terms: "terms", tokens: "tokens" });
  let openSection = $derived(sectionByTab[tab]);
  let openOutsideNote = $derived(outsideNote(settings, openSection));

  // Запуск этой панели. Настройки и кнопки отключает любой запуск (`llmRun.active`)
  let isProcessing = $state(false);
  let progressStep = $state("");
  let progress = $state<{ batch: number; total: number } | null>(null);
  let stopRequested = $state(false);
  let errorMessage = $state<string | null>(null);
  let doneMessage = $state("");

  $effect(() => { descriptionsOpen = panelOpen && sectionByTab.characters === "descriptions"; });
  // Сообщение относится к разделу, в котором появилось
  $effect(() => {
    void openSection;
    errorMessage = null;
    doneMessage = "";
  });

  // Панель закрывается вместе с глоссарием: результат запроса показать будет негде
  onDestroy(() => {
    if (isProcessing) void invoke("cancel_llm_run");
  });

  function beginRun(step: string) {
    if (!beginLlmRun(step)) return false;
    isProcessing = true;
    errorMessage = null;
    doneMessage = "";
    progress = null;
    stopRequested = false;
    progressStep = step;
    return true;
  }

  function endRun() {
    isProcessing = false;
    endLlmRun();
  }

  /** Канал итогов пакетов: записи появляются в панели сразу, как только их вернула модель. */
  function batchChannel<T>(onItems: (items: T[]) => void) {
    const channel = new Channel<BatchProgress<T>>();
    channel.onmessage = (message) => {
      progress = { batch: message.batch, total: message.total_batches };
      onItems(message.items);
    };
    return channel;
  }

  async function stopRun() {
    stopRequested = true;
    progressStep = "Останавливаемся: дожидаемся ответа на текущий пакет...";
    await invoke("cancel_llm_run");
  }

  function failedBatchesMessage(count: number) {
    return `Часть данных модель не обработала (пакетов с ошибкой: ${count}). Эти записи остались без изменений, запустите обработку ещё раз.`;
  }

  // Имена
  const characterName = (c: PanelCharacter) => c.original_name.trim() || c.alias;
  let unselectedAliases = $state<string[]>([]);
  let nameTargets = $derived(characters.filter((c) => !unselectedAliases.includes(c.alias)));
  let reviewNames = $state<{ alias: string; original: string; current: string; translated: string; origin: string; selected: boolean }[]>([]);
  let unchangedNames = $state(0);
  let namesRequested = $state(0);
  let namesFinished = $state(false);
  let chosenNames = $derived(reviewNames.filter((row) => row.selected && row.translated.trim()));
  // Итог команды может опередить последний пакет из канала, поэтому подпись считается по строкам
  let namesNote = $derived.by(() => {
    if (!namesFinished) return "Новые написания появляются здесь по мере ответа модели.";
    const missing = namesRequested - reviewNames.length - unchangedNames;
    const note = `Другое написание: ${reviewNames.length}, без изменений: ${unchangedNames}`;
    return missing > 0 ? `${note}, не получено ответов: ${missing}.` : `${note}.`;
  });

  /** Модель пишет имена выбранных персонажей, видя весь состав игры. Заполненные имена тоже участвуют. */
  async function startNames() {
    const chosen = nameTargets;
    if (!beginRun("Модель определяет происхождение имён...")) return;
    reviewNames = [];
    unchangedNames = 0;
    namesRequested = chosen.length;
    namesFinished = false;

    try {
      const result = await invoke<AutofillResult<NameSuggestion>>("translate_character_names", {
        target: targetFor(settings, "names"),
        names: chosen.map(characterName),
        cast: characters.flatMap((c) => [characterName(c), ...c.merged.map((m) => m.original_name)]),
        onBatch: batchChannel<NameSuggestion>((items) => {
          for (const item of items) {
            // Модель получает повторы один раз, а написание нужно каждому коду с этим именем
            for (const character of chosen.filter((c) => characterName(c).toLowerCase() === item.original.toLowerCase())) {
              const current = character.translated_name.trim();
              if (current === item.translated) {
                unchangedNames += 1;
              } else {
                reviewNames.push({
                  alias: character.alias,
                  original: characterName(character),
                  current,
                  translated: item.translated,
                  origin: item.origin,
                  selected: true
                });
              }
            }
          }
        })
      });
      namesFinished = true;
      if (result.failed_batches > 0) errorMessage = failedBatchesMessage(result.failed_batches);
    } catch (e) {
      namesRequested = 0;
      errorMessage = `Не удалось перевести имена: ${String(e)}`;
    } finally {
      endRun();
    }
  }

  function applyNames() {
    onnames(chosenNames.map(({ alias, translated }) => ({ alias, translated: translated.trim() })));
    doneMessage = `Обновлено имён: ${chosenNames.length}. Описания сохранят старое написание, пока их не обновить.`;
    reviewNames = [];
    namesRequested = 0;
  }

  // Перевод терминов
  // Без явного выбора в запуске непереведённые: готовый перевод нельзя заменить случайно
  let unselectedTerms = $state<string[] | null>(null);
  let termsDeselected = $derived(unselectedTerms ?? terms.filter((t) => t.translated_term.trim()).map((t) => t.id));
  let termTargets = $derived(terms.filter((t) => !termsDeselected.includes(t.id)));
  let reviewTranslations = $state<{ id: string; source: string; current: string; translated: string; category: string; notes: string; selected: boolean }[]>([]);
  let translationsRun = $state(false);
  let chosenTranslations = $derived(reviewTranslations.filter((row) => row.selected && row.translated.trim()));

  async function startTerms() {
    if (!beginRun("Модель переводит термины...")) return;
    reviewTranslations = [];
    translationsRun = true;
    try {
      const result = await invoke<AutofillResult<PanelTerm>>("autofill_glossary_terms", {
        target: targetFor(settings, "terms"),
        terms: $state.snapshot(termTargets),
        onBatch: batchChannel<PanelTerm>((items) => {
          for (const term of items) {
            const known = terms.find((t) => t.id === term.id);
            if (!known || reviewTranslations.some((row) => row.id === term.id)) continue;
            if (known.translated_term === term.translated_term && known.category === term.category) continue;
            reviewTranslations.push({
              id: term.id,
              source: term.source_term,
              current: known.translated_term,
              translated: term.translated_term,
              category: term.category,
              notes: term.notes,
              selected: true
            });
          }
        })
      });
      if (result.failed_batches > 0) errorMessage = failedBatchesMessage(result.failed_batches);
    } catch (e) {
      errorMessage = `Не удалось обработать термины: ${String(e)}`;
    } finally {
      endRun();
    }
  }

  function applyTranslations() {
    onterms(chosenTranslations.map(({ id, translated, category, notes }) => ({ id, translated: translated.trim(), category, notes })));
    doneMessage = `Обновлено терминов: ${chosenTranslations.length}`;
    reviewTranslations = [];
    translationsRun = false;
  }

  // Токены без реплик модели показать нечего. Без явного выбора в запуске незаполненные, как у терминов
  let readableTokens = $derived(tokens.filter((t) => t.lines > 0));
  let unselectedTokens = $state<string[] | null>(null);
  let tokensDeselected = $derived(unselectedTokens ?? readableTokens.filter((t) => t.value.trim() || t.note.trim()).map((t) => t.name));
  let tokenTargets = $derived(readableTokens.filter((t) => !tokensDeselected.includes(t.name)));
  let reviewTokens = $state<(TokenSuggestion & { lines: number; selected: boolean })[]>([]);
  let tokensRun = $state(false);
  let chosenTokens = $derived(reviewTokens.filter((t) => t.selected && (t.value.trim() || t.note.trim())));

  async function startTokens() {
    const names = tokenTargets.map((t) => t.name);
    if (!beginRun("Модель читает реплики с токенами...")) return;
    reviewTokens = [];
    tokensRun = true;
    try {
      const result = await invoke<AutofillResult<TokenSuggestion>>("suggest_glossary_tokens", {
        gameDir,
        target: targetFor(settings, "tokens"),
        names,
        hints: $state.snapshot(hints),
        onBatch: batchChannel<TokenSuggestion>((items) => {
          reviewTokens.push(...items.map((item) => ({
            ...item,
            lines: tokens.find((t) => t.name === item.name)?.lines ?? 0,
            selected: true
          })));
        })
      });
      if (result.failed_batches > 0) errorMessage = failedBatchesMessage(result.failed_batches);
    } catch (e) {
      errorMessage = `Не удалось обновить пояснения: ${String(e)}`;
    } finally {
      endRun();
    }
  }

  function applyTokens() {
    ontokens(chosenTokens.map(({ name, value, note }) => ({ name, value: value.trim(), note: note.trim() })));
    doneMessage = `Обновлено токенов: ${chosenTokens.length}`;
    reviewTokens = [];
    tokensRun = false;
  }
</script>

{#snippet runButton(label: string, disabled: boolean, onclick: () => void)}
  <button class="btn-run" {onclick} disabled={disabled || llmRun.active}>
    <Sparkles size={13} />
    <span>{label}</span>
  </button>
{/snippet}

{#snippet categorySelect(row: { category: string }, label: string)}
  <select class="input" bind:value={row.category} aria-label={`Категория термина ${label}`}>
    <option value="general">Общее</option>
    <option value="location">Локация</option>
    <option value="title">Титул</option>
    <option value="item">Предмет</option>
  </select>
{/snippet}

<div class="ai-panel">
  <div class="ai-bar">
    <button class="ai-toggle" aria-expanded={panelOpen} onclick={() => panelOpen = !panelOpen}>
      {#if panelOpen}
        <ChevronDown size={15} />
      {:else}
        <ChevronRight size={15} />
      {/if}
      <Sparkles size={14} class="ai-bar-icon" />
      <span class="ai-title">Обработка через LLM</span>
    </button>
    <div class="ai-status">
      {#if isProcessing}
        <Loader2 size={13} class="spin" />
        <span class="ai-status-text">{progressStep}</span>
        {#if progress}
          <span class="ai-count">Пакет {progress.batch} из {progress.total}</span>
        {/if}
        <button class="btn-stop" onclick={stopRun} disabled={stopRequested}>
          <Square size={11} />
          <span>Остановить</span>
        </button>
      {:else if llmRun.active}
        <Loader2 size={13} class="spin" />
        <span class="ai-status-text">{llmRun.note}</span>
      {/if}
    </div>
  </div>

  {#if panelOpen}
    <div class="ai-tabs">
      {#each LLM_ACTIONS.filter(({ id }) => SECTIONS[tab].includes(id)) as section (section.id)}
        <button
          class="ai-tab"
          class:active={openSection === section.id}
          onclick={() => { sectionByTab[tab] = section.id; }}
        >{section.label}</button>
      {/each}
      <div class="ai-choice">
        <LlmChoice action={openSection} {settings} {onselect} />
      </div>
    </div>
    <div class="ai-body">
      {#if openOutsideNote}
        <p class="ai-outside">{openOutsideNote}</p>
      {/if}
      {#if errorMessage}
        <div class="ai-error">
          <AlertCircle size={14} />
          <span>{errorMessage}</span>
        </div>
      {/if}
      {#if doneMessage}
        <p class="ai-done"><Check size={13} /> {doneMessage}</p>
      {/if}

      {#if openSection === "descriptions"}
        {@render descriptions()}
      {:else if openSection === "names"}
        <p class="ai-note">Модель увидит все имена игры, определит их происхождение и заново напишет имена выбранных персонажей, включая заполненные. Что заменить, выберете вы.</p>
        <EntryPicker
          items={characters.map((c) => ({ id: c.alias, name: characterTitle(c) }))}
          unselected={unselectedAliases}
          disabled={llmRun.active}
          onchange={(next) => unselectedAliases = next}
          onempty={() => unselectedAliases = characters.filter((c) => c.translated_name.trim()).map((c) => c.alias)}
        />
        {@render runButton(`Перевести имена (${nameTargets.length})`, nameTargets.length === 0, startNames)}

        {#if namesRequested > 0}
          <div class="ai-review">
            <div class="ai-review-head">
              <span class="ai-note">{namesNote}</span>
              {#if reviewNames.length > 0}
                <div class="ai-toggles">
                  <button class="btn-link" onclick={() => { for (const row of reviewNames) row.selected = true; }}>Выбрать все</button>
                  <button class="btn-link" onclick={() => { for (const row of reviewNames) row.selected = false; }}>Снять все</button>
                </div>
              {/if}
            </div>
            {#each reviewNames as row (row.alias)}
              <div class="ai-row" class:off={!row.selected}>
                <input type="checkbox" bind:checked={row.selected} aria-label={`Заменить имя ${row.original}`} />
                <div class="ai-source">
                  <span class="ai-term">{row.original} <span class="ai-count">{row.origin}</span></span>
                  <span class="ai-example">Сейчас: {row.current || "нет перевода"}</span>
                </div>
                <input type="text" class="input" bind:value={row.translated} aria-label={`Новое имя ${row.original}`} />
              </div>
            {/each}
            {#if reviewNames.length > 0}
              <button class="btn-apply" onclick={applyNames} disabled={chosenNames.length === 0 || isProcessing}>
                <Check size={14} />
                <span>Заменить выбранные ({chosenNames.length})</span>
              </button>
            {/if}
          </div>
        {/if}
      {:else if openSection === "terms"}
        <EntryPicker
          items={terms.map((t) => ({ id: t.id, name: t.source_term }))}
          unselected={termsDeselected}
          disabled={llmRun.active}
          onchange={(next) => unselectedTerms = next}
          onempty={() => unselectedTerms = null}
        />
        {@render runButton(`Перевести термины (${termTargets.length})`, termTargets.length === 0, startTerms)}

        {#if translationsRun && !isProcessing && reviewTranslations.length === 0 && !errorMessage}
          <p class="ai-note">Модель не предложила изменений.</p>
        {/if}
        {#if reviewTranslations.length > 0}
          <div class="ai-review">
            {#each reviewTranslations as row (row.id)}
              <div class="ai-row ai-row-select" class:off={!row.selected}>
                <input type="checkbox" bind:checked={row.selected} aria-label={`Заменить перевод ${row.source}`} />
                <div class="ai-source">
                  <span class="ai-term">{row.source}</span>
                  <span class="ai-example">Сейчас: {row.current || "нет перевода"}</span>
                </div>
                <input type="text" class="input" bind:value={row.translated} aria-label={`Перевод термина ${row.source}`} />
                {@render categorySelect(row, row.source)}
              </div>
            {/each}
            <button class="btn-apply" onclick={applyTranslations} disabled={chosenTranslations.length === 0 || isProcessing}>
              <Check size={14} />
              <span>Заменить выбранные ({chosenTranslations.length})</span>
            </button>
          </div>
        {/if}
      {:else if openSection === "tokens"}
        <p class="ai-note">Модель читает реплики с токеном, не видя его имени, и предлагает пояснение и значение. Что записать в глоссарий, выберете вы.</p>
        <EntryPicker
          items={readableTokens.map((t) => ({ id: t.name, name: `[${t.name}]` }))}
          unselected={tokensDeselected}
          disabled={llmRun.active}
          onchange={(next) => unselectedTokens = next}
          onempty={() => unselectedTokens = null}
        />
        {@render runButton(`Обновить пояснения (${tokenTargets.length})`, tokenTargets.length === 0, startTokens)}

        {#if tokensRun && !isProcessing && reviewTokens.length === 0 && !errorMessage}
          <p class="ai-note">Модель не смогла ничего предложить.</p>
        {/if}
        {#if reviewTokens.length > 0}
          <div class="ai-review">
            {#each reviewTokens as row (row.name)}
              <div class="ai-row ai-row-token" class:off={!row.selected}>
                <input type="checkbox" bind:checked={row.selected} aria-label={`Записать токен ${row.name}`} />
                <div class="ai-source">
                  <span class="ai-term">[{row.name}]</span>
                  <span class="ai-example">Реплик: {row.lines}</span>
                </div>
                <input type="text" class="input" bind:value={row.value} placeholder="Значение" aria-label={`Значение токена ${row.name}`} />
                <input type="text" class="input" bind:value={row.note} placeholder="Пояснение" aria-label={`Пояснение токена ${row.name}`} />
              </div>
            {/each}
            <button class="btn-apply" onclick={applyTokens} disabled={chosenTokens.length === 0 || isProcessing}>
              <Check size={14} />
              <span>Записать выбранные ({chosenTokens.length})</span>
            </button>
          </div>
        {/if}
      {/if}
    </div>
  {/if}
</div>

<style>
  /* Цвет запусков модели, как у кнопки обновления персонажа: панель не должна сливаться с вкладками и списком */
  .ai-panel {
    flex-shrink: 0;
    margin: 10px 16px;
    overflow: hidden;
    color-scheme: dark;
    border: 1px solid rgba(99, 102, 241, 0.4);
    border-left: 3px solid #6366f1;
    border-radius: 8px;
    background: #12122a;
    box-shadow: 0 8px 20px -12px rgba(0, 0, 0, 0.9);
    font-size: 12px;
    color: #cbd5e1;
  }

  .ai-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 40px;
    padding-right: 12px;
    background: #26245c;
  }

  .ai-toggle {
    display: flex;
    flex: 1;
    align-items: center;
    align-self: stretch;
    gap: 8px;
    padding: 6px 12px;
    border: none;
    background: transparent;
    color: #a5b4fc;
    cursor: pointer;
  }

  .ai-toggle:hover .ai-title {
    color: #fff;
  }

  .ai-toggle :global(.ai-bar-icon) {
    flex-shrink: 0;
  }

  .ai-title {
    font-size: 13px;
    font-weight: 600;
    color: #e0e7ff;
    white-space: nowrap;
  }

  /* Выбранная вкладка одного цвета с содержимым и закрывает линию под полосой, как корешок папки */
  .ai-tabs {
    display: flex;
    align-items: flex-end;
    gap: 4px;
    padding: 8px 12px 0;
    border-bottom: 1px solid rgba(99, 102, 241, 0.45);
  }

  .ai-tab {
    margin-bottom: -1px;
    padding: 6px 14px;
    border: 1px solid transparent;
    border-radius: 6px 6px 0 0;
    background: rgba(148, 163, 184, 0.08);
    font-size: 12px;
    font-weight: 500;
    color: #94a3b8;
    white-space: nowrap;
    cursor: pointer;
  }

  .ai-tab:hover {
    color: #f1f5f9;
  }

  .ai-tab.active {
    border-color: rgba(99, 102, 241, 0.45);
    border-top-color: #818cf8;
    border-bottom-color: #0b0b1a;
    background: #0b0b1a;
    color: #fff;
  }

  .ai-status {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    margin-left: auto;
  }

  .ai-status-text {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .ai-count {
    font-size: 11px;
    color: #94a3b8;
    white-space: nowrap;
  }

  .ai-body {
    display: flex;
    flex-direction: column;
    gap: 10px;
    max-height: 42vh;
    overflow-y: auto;
    padding: 10px 12px 12px;
    background: #0b0b1a;
  }

  .ai-choice {
    flex: 0 1 26.5rem;
    min-width: 0;
    margin-left: auto;
    padding: 0 0 5px 12px;
  }

  .ai-note,
  .ai-outside {
    margin: 0;
    font-size: 11px;
    color: #94a3b8;
  }

  .ai-outside {
    color: #fbbf24;
  }

  .ai-done {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    color: #34d399;
  }

  .ai-error {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border: 1px solid rgba(244, 63, 94, 0.3);
    border-radius: 6px;
    background: rgba(244, 63, 94, 0.1);
    color: #fecdd3;
  }

  .ai-toggles {
    display: flex;
    flex-shrink: 0;
    gap: 10px;
    margin: 4px 0;
  }

  .btn-link {
    padding: 0;
    border: none;
    background: transparent;
    font-size: 11px;
    color: #a5b4fc;
    cursor: pointer;
  }

  .btn-link:hover:not(:disabled) {
    color: #e0e7ff;
  }

  .btn-link:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .btn-run,
  .btn-apply {
    display: flex;
    align-items: center;
    align-self: flex-start;
    gap: 6px;
    padding: 6px 14px;
    border: 1px solid rgba(52, 211, 153, 0.5);
    border-radius: 6px;
    background: #059669;
    font-size: 12px;
    font-weight: 600;
    color: #fff;
    cursor: pointer;
  }

  /* Запуск выглядит как обновление групп персонажей, а зелёным остаётся запись в глоссарий */
  .btn-run {
    border-color: #4338ca;
    background: #1e1b4b;
    color: #c7d2fe;
  }

  .btn-run:hover:not(:disabled) {
    border-color: #6366f1;
    background: #312e81;
    color: #fff;
  }

  .btn-apply:hover:not(:disabled) {
    background: #047857;
  }

  .btn-run:disabled,
  .btn-apply:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-apply {
    align-self: flex-end;
    margin-top: 4px;
  }

  .btn-stop {
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 3px 10px;
    border: 1px solid rgba(71, 85, 105, 0.6);
    border-radius: 6px;
    background: rgba(51, 65, 85, 0.6);
    font-size: 11px;
    color: #cbd5e1;
    white-space: nowrap;
    cursor: pointer;
  }

  .btn-stop:hover:not(:disabled) {
    color: #f8fafc;
  }

  .btn-stop:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .ai-review {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px;
    border: 1px solid #1e2638;
    border-radius: 8px;
    background: rgba(15, 23, 42, 0.6);
  }

  .ai-review-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .ai-row {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) minmax(0, 1fr);
    align-items: center;
    gap: 8px;
  }

  .ai-row-select {
    grid-template-columns: auto minmax(0, 1.2fr) minmax(0, 1fr) 7rem;
  }

  .ai-row-token {
    grid-template-columns: auto minmax(0, 0.8fr) minmax(0, 0.8fr) minmax(0, 1.6fr);
  }

  .ai-row.off {
    opacity: 0.5;
  }

  .ai-source {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .ai-term {
    color: #f1f5f9;
  }

  .ai-example {
    overflow: hidden;
    font-size: 11px;
    color: #94a3b8;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .input {
    min-width: 0;
    padding: 5px 9px;
    border: 1px solid rgba(51, 65, 85, 0.8);
    border-radius: 6px;
    outline: none;
    background: #090d16;
    font-size: 12px;
    color: #f1f5f9;
  }

  .input:focus {
    border-color: #10b981;
  }
</style>
