<script lang="ts">
  import { onDestroy, type Snippet } from "svelte";
  import { Channel, invoke } from "@tauri-apps/api/core";
  import { AlertCircle, Check, Loader2, Sparkles, Square } from "lucide-svelte";
  import RequestSlider from "./RequestSlider.svelte";
  import { actionLabel, targetFor, type LlmAction, type LlmSettings } from "./llm-settings";

  interface PanelCharacter {
    alias: string;
    original_name: string;
    translated_name: string;
    merged: { alias: string; original_name: string }[];
  }

  interface PanelTerm {
    id: string;
    source_term: string;
    translated_term: string;
    category: string;
    notes: string;
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

  interface ReviewedTerm extends PanelTerm {
    count: number;
    example: string;
  }

  interface ExtractedTerms {
    items: ReviewedTerm[];
    failed_batches: number;
    candidates: number;
    single_words_skipped: boolean;
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
    locked = false,
    lockedNote = "",
    busy = $bindable(false),
    descriptionsOpen = $bindable(false),
    descriptions,
    onnames,
    onterms,
    onaddterms,
    ontokens
  }: {
    tab: GlossaryTab;
    gameDir: string;
    characters: PanelCharacter[];
    terms: PanelTerm[];
    tokens: PanelToken[];
    hints: object;
    settings: LlmSettings;
    locked?: boolean;
    lockedNote?: string;
    busy?: boolean;
    descriptionsOpen?: boolean;
    descriptions: Snippet;
    onnames: (rows: { alias: string; translated: string }[]) => void;
    onterms: (rows: { id: string; translated: string; category: string; notes: string }[]) => void;
    onaddterms: (terms: PanelTerm[]) => void;
    ontokens: (rows: { name: string; value: string; note: string }[]) => void;
  } = $props();

  const SECTIONS: Record<GlossaryTab, { id: LlmAction; label: string }[]> = {
    characters: [
      { id: "descriptions", label: "Описания персонажей" },
      { id: "names", label: "Имена" }
    ],
    terms: [
      { id: "terms", label: "Перевод терминов" },
      { id: "termSearch", label: "Поиск в тексте игры" }
    ],
    tokens: [{ id: "tokens", label: "Пояснения токенов" }]
  };
  // Размеры пакетов те же, что в llm.rs: так ползунок считает настоящие запросы
  const TERMS_PER_REQUEST = 20;
  const TOKENS_PER_REQUEST = 10;
  const MAX_TERM_REQUESTS = 10;

  let openByTab = $state<Record<GlossaryTab, LlmAction | null>>({ characters: null, terms: null, tokens: null });
  let openSection = $derived(openByTab[tab]);

  let isProcessing = $state(false);
  let progressStep = $state("");
  let progress = $state<{ batch: number; total: number } | null>(null);
  let stopRequested = $state(false);
  let errorMessage = $state<string | null>(null);
  let doneMessage = $state("");

  $effect(() => { busy = isProcessing; });
  $effect(() => { descriptionsOpen = openByTab.characters === "descriptions"; });
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
    isProcessing = true;
    errorMessage = null;
    doneMessage = "";
    progress = null;
    stopRequested = false;
    progressStep = step;
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
    const chosen = characters.filter((c) => !unselectedAliases.includes(c.alias));
    if (chosen.length === 0) {
      errorMessage = "Нет персонажей для обработки: выберите хотя бы одного.";
      return;
    }
    beginRun("Модель определяет происхождение имён...");
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
      errorMessage = `Не удалось обновить имена: ${String(e)}`;
    } finally {
      isProcessing = false;
    }
  }

  function applyNames() {
    onnames(chosenNames.map(({ alias, translated }) => ({ alias, translated: translated.trim() })));
    doneMessage = `Обновлено имён: ${chosenNames.length}. Описания сохранят старое написание, пока их не обновить.`;
    reviewNames = [];
    namesRequested = 0;
  }

  // Перевод терминов
  let onlyEmptyTerms = $state(true);
  let reviewTranslations = $state<{ id: string; source: string; current: string; translated: string; category: string; notes: string; selected: boolean }[]>([]);
  let translationsRun = $state(false);
  let chosenTranslations = $derived(reviewTranslations.filter((row) => row.selected && row.translated.trim()));

  async function startTerms() {
    beginRun("Модель переводит термины...");
    reviewTranslations = [];
    translationsRun = true;
    try {
      const result = await invoke<AutofillResult<PanelTerm>>("autofill_glossary_terms", {
        target: targetFor(settings, "terms"),
        terms: $state.snapshot(terms),
        onlyEmpty: onlyEmptyTerms,
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
      isProcessing = false;
    }
  }

  function applyTranslations() {
    onterms(chosenTranslations.map(({ id, translated, category, notes }) => ({ id, translated: translated.trim(), category, notes })));
    doneMessage = `Обновлено терминов: ${chosenTranslations.length}`;
    reviewTranslations = [];
    translationsRun = false;
  }

  // Поиск терминов
  let termRequests = $state(3);
  let foundTerms = $state<(ReviewedTerm & { selected: boolean })[]>([]);
  let foundNote = $state("");
  let chosenFound = $derived(foundTerms.filter((t) => t.selected && t.translated_term.trim()));

  async function startTermSearch() {
    beginRun("Поиск названий по всему тексту игры...");
    foundTerms = [];
    foundNote = "";
    try {
      const result = await invoke<ExtractedTerms>("extract_glossary_terms_from_game", {
        gameDir,
        target: targetFor(settings, "termSearch"),
        maxCandidates: termRequests * TERMS_PER_REQUEST,
        knownTerms: terms.map((t) => t.source_term),
        onBatch: batchChannel<ReviewedTerm>((items) => {
          foundTerms.push(...items.map((item) => ({ ...item, selected: true })));
          if (!stopRequested) progressStep = "Модель проверяет кандидатов...";
        })
      });
      foundNote = result.candidates === 0
        ? "В тексте игры не нашлось новых названий."
        : `Модель оставила ${result.items.length} из ${result.candidates} кандидатов.`;
      if (stopRequested) foundNote = `Остановлено. Модель оставила ${result.items.length} терминов.`;
      if (result.single_words_skipped) {
        foundNote += " В языке игры заглавные пишутся у всех существительных, поэтому искались только фразы из нескольких слов.";
      }
      if (result.failed_batches > 0) {
        errorMessage = `Часть кандидатов модель не обработала (пакетов с ошибкой: ${result.failed_batches}). Запустите поиск ещё раз.`;
      }
    } catch (e) {
      errorMessage = `Не удалось найти термины: ${String(e)}`;
    } finally {
      isProcessing = false;
    }
  }

  function addFoundTerms() {
    onaddterms(chosenFound.map(({ id, source_term, translated_term, category, notes }) => ({ id, source_term, translated_term, category, notes })));
    doneMessage = `Добавлено терминов: ${chosenFound.length}`;
    foundTerms = [];
    foundNote = "";
  }

  // Токены: они идут от самых частых, поэтому меньшее число запросов берёт важные
  let onlyEmptyTokens = $state(true);
  let tokenRequests = $state(1);
  let tokenTargets = $derived(tokens.filter((t) => t.lines > 0 && (!onlyEmptyTokens || (!t.value.trim() && !t.note.trim()))));
  let maxTokenRequests = $derived(Math.max(Math.ceil(tokenTargets.length / TOKENS_PER_REQUEST), 1));
  let tokenRequestsShown = $derived(Math.min(tokenRequests, maxTokenRequests));
  let reviewTokens = $state<(TokenSuggestion & { lines: number; selected: boolean })[]>([]);
  let tokensRun = $state(false);
  let chosenTokens = $derived(reviewTokens.filter((t) => t.selected && (t.value.trim() || t.note.trim())));

  async function startTokens() {
    const names = tokenTargets.slice(0, tokenRequestsShown * TOKENS_PER_REQUEST).map((t) => t.name);
    beginRun("Модель читает реплики с токенами...");
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
      errorMessage = `Не удалось разобрать токены: ${String(e)}`;
    } finally {
      isProcessing = false;
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
  <button class="btn-run" {onclick} disabled={disabled || isProcessing || locked}>
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
    <Sparkles size={14} class="ai-bar-icon" />
    {#each SECTIONS[tab] as section (section.id)}
      <button
        class="ai-pill"
        class:active={openSection === section.id}
        onclick={() => { openByTab[tab] = openSection === section.id ? null : section.id; }}
      >{section.label}</button>
    {/each}
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
      {:else if lockedNote}
        <Loader2 size={13} class="spin" />
        <span class="ai-status-text">{lockedNote}</span>
      {/if}
    </div>
  </div>

  {#if openSection}
    <div class="ai-body">
      <p class="ai-model">Модель: {actionLabel(settings, openSection)}</p>

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
        <details class="ai-picker">
          <summary>Выбрано: {characters.length - unselectedAliases.length} из {characters.length}</summary>
          <div class="ai-toggles">
            <button class="btn-link" onclick={() => unselectedAliases = []}>Выбрать всех</button>
            <button class="btn-link" onclick={() => unselectedAliases = characters.map((c) => c.alias)}>Снять всех</button>
          </div>
          <div class="ai-picker-grid">
            {#each characters as character (character.alias)}
              <label class="ai-check">
                <input
                  type="checkbox"
                  checked={!unselectedAliases.includes(character.alias)}
                  disabled={isProcessing}
                  onchange={(e) => {
                    unselectedAliases = e.currentTarget.checked
                      ? unselectedAliases.filter((alias) => alias !== character.alias)
                      : [...unselectedAliases, character.alias];
                  }}
                />
                <span>{characterName(character)}</span>
              </label>
            {/each}
          </div>
        </details>
        {@render runButton("Обновить имена всей группой", characters.length === 0, startNames)}

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
        <label class="ai-check">
          <input type="checkbox" bind:checked={onlyEmptyTerms} disabled={isProcessing} />
          <span>Только непереведённые термины</span>
        </label>
        {@render runButton("Перевести термины", terms.length === 0, startTerms)}

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
      {:else if openSection === "termSearch"}
        <RequestSlider
          id="term-requests"
          label="Запросов"
          status={`${termRequests} из ${MAX_TERM_REQUESTS}`}
          min={1}
          max={MAX_TERM_REQUESTS}
          value={termRequests}
          disabled={isProcessing}
          oninput={(event) => termRequests = event.currentTarget.valueAsNumber}
        >
          {#snippet ends()}
            <span>{TERMS_PER_REQUEST} частых названий</span>
            <span>{MAX_TERM_REQUESTS * TERMS_PER_REQUEST} названий, долго</span>
          {/snippet}
        </RequestSlider>
        {@render runButton("Найти термины в игре", false, startTermSearch)}

        {#if foundNote || foundTerms.length > 0}
          <div class="ai-review">
            <div class="ai-review-head">
              <span class="ai-note">{foundNote || "Найденные термины появляются здесь по мере проверки."}</span>
              {#if foundTerms.length > 0}
                <div class="ai-toggles">
                  <button class="btn-link" onclick={() => { for (const term of foundTerms) term.selected = true; }}>Выбрать все</button>
                  <button class="btn-link" onclick={() => { for (const term of foundTerms) term.selected = false; }}>Снять все</button>
                </div>
              {/if}
            </div>
            {#each foundTerms as term (term.id)}
              <div class="ai-row ai-row-select" class:off={!term.selected}>
                <input type="checkbox" bind:checked={term.selected} aria-label={`Добавить термин ${term.source_term}`} />
                <div class="ai-source">
                  <span class="ai-term">{term.source_term} <span class="ai-count">× {term.count}</span></span>
                  <span class="ai-example" title={term.example}>{term.example}</span>
                </div>
                <input type="text" class="input" bind:value={term.translated_term} aria-label={`Перевод термина ${term.source_term}`} />
                {@render categorySelect(term, term.source_term)}
              </div>
            {/each}
            {#if foundTerms.length > 0}
              <button class="btn-apply" onclick={addFoundTerms} disabled={chosenFound.length === 0 || isProcessing}>
                <Check size={14} />
                <span>Добавить выбранные ({chosenFound.length})</span>
              </button>
            {/if}
          </div>
        {/if}
      {:else if openSection === "tokens"}
        <p class="ai-note">Модель читает реплики с токеном, не видя его имени, и предлагает пояснение и значение. Что записать в глоссарий, выберете вы.</p>
        <label class="ai-check">
          <input type="checkbox" bind:checked={onlyEmptyTokens} disabled={isProcessing} />
          <span>Только незаполненные</span>
        </label>
        <RequestSlider
          id="token-requests"
          label="Запросов"
          status={tokenTargets.length === 0 ? "нет токенов" : `${tokenRequestsShown} из ${maxTokenRequests}`}
          min={1}
          max={Math.max(maxTokenRequests, 2)}
          value={tokenRequestsShown}
          disabled={isProcessing || maxTokenRequests < 2}
          oninput={(event) => tokenRequests = event.currentTarget.valueAsNumber}
        >
          {#snippet ends()}
            <span>самые частые</span>
            <span>токенов: {tokenTargets.length}</span>
          {/snippet}
        </RequestSlider>
        {@render runButton("Разобрать токены", tokenTargets.length === 0, startTokens)}

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
  .ai-panel {
    flex-shrink: 0;
    border-bottom: 1px solid #1a2232;
    background: #0d0f16;
    font-size: 12px;
    color: #cbd5e1;
  }

  .ai-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 40px;
    padding: 6px 16px;
  }

  .ai-bar :global(.ai-bar-icon) {
    flex-shrink: 0;
    color: #a78bfa;
  }

  .ai-pill {
    padding: 4px 12px;
    border: 1px solid #1e2638;
    border-radius: 999px;
    background: transparent;
    font-size: 12px;
    color: #94a3b8;
    white-space: nowrap;
    cursor: pointer;
  }

  .ai-pill:hover {
    color: #f1f5f9;
    border-color: #334155;
  }

  .ai-pill.active {
    color: #34d399;
    border-color: rgba(16, 185, 129, 0.5);
    background: rgba(16, 185, 129, 0.08);
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
    padding: 4px 16px 12px;
  }

  .ai-model,
  .ai-note {
    margin: 0;
    font-size: 11px;
    color: #64748b;
  }

  .ai-note {
    color: #94a3b8;
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

  .ai-check {
    display: flex;
    align-items: center;
    gap: 6px;
    cursor: pointer;
  }

  .ai-picker summary {
    color: #94a3b8;
    cursor: pointer;
  }

  .ai-picker-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: 4px 12px;
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
    color: #34d399;
    cursor: pointer;
  }

  .btn-link:hover {
    color: #6ee7b7;
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

  .btn-run:hover:not(:disabled),
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
