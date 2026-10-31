<script lang="ts">
  import { onMount } from "svelte";
  import { Channel, invoke } from "@tauri-apps/api/core";
  import {
    Sparkles,
    Bot,
    Server,
    Users,
    BookMarked,
    Check,
    AlertCircle,
    Loader2,
    RefreshCw,
    X,
    Sliders,
    ArrowRight,
    Wand2,
    Search,
    Square
  } from "lucide-svelte";

  export interface CharacterGlossaryEntry {
    alias: string;
    original_name: string;
    translated_name: string;
    gender: string;
    notes: string;
  }

  export interface TermGlossaryEntry {
    id: string;
    source_term: string;
    translated_term: string;
    category: string;
    notes: string;
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

  interface CharacterResult extends CharacterGlossaryEntry {
    samples: string[];
  }

  interface ResultRow {
    index: number;
    samples: string[];
    done: boolean;
  }

  interface ReviewedTerm extends TermGlossaryEntry {
    count: number;
    example: string;
  }

  interface ExtractedTerms {
    items: ReviewedTerm[];
    failed_batches: number;
    candidates: number;
    single_words_skipped: boolean;
  }

  let {
    show = false,
    gameDir = "",
    characters = [],
    terms = [],
    initialTab = "characters",
    onapply = () => {},
    onclose = () => {}
  }: {
    show: boolean;
    gameDir: string;
    characters: CharacterGlossaryEntry[];
    terms: TermGlossaryEntry[];
    initialTab?: "characters" | "terms";
    onapply: (updatedChars: CharacterGlossaryEntry[], updatedTerms: TermGlossaryEntry[]) => void;
    onclose: () => void;
  } = $props();

  const LLM_API_KEY = "translator.llm_api_url";
  const LLM_MODEL_KEY = "translator.llm_model";

  let activeTab = $state<"characters" | "terms">("characters");
  let apiUrl = $state("http://127.0.0.1:1234/v1");
  let selectedModel = $state("mistral-small");
  let availableModels = $state<string[]>([]);
  let testingConnection = $state(false);
  let connectionStatus = $state<"idle" | "success" | "error">("idle");
  let connectionMsg = $state("");

  // Настройки генерации
  let onlyEmpty = $state(true);
  let extractTermsFromGame = $state(false);
  let maxCandidates = $state(60);

  // Состояние выполнения
  let isProcessing = $state(false);
  let progressStep = $state("");
  let errorMessage = $state<string | null>(null);
  let progress = $state<{ batch: number; total: number } | null>(null);
  let stopRequested = $state(false);
  // Итог команды может опередить последний пакет из канала, поэтому подпись считается по строкам
  let finishedRun = $state<{ noun: string; rows: () => ResultRow[] } | null>(null);
  let resultLabel = $derived.by(() => {
    if (!finishedRun) return progressStep;
    const rows = finishedRun.rows();
    const done = rows.filter((r) => r.done).length;
    return `${stopRequested ? "Остановлено." : "Готово!"} ${finishedRun.noun}: ${done} из ${rows.length}`;
  });

  // Строки результата: значения лежат в previewChars и previewTerms, поэтому их можно править
  let characterRows = $state<ResultRow[]>([]);
  let termRows = $state<ResultRow[]>([]);

  // Результаты генерации для предпросмотра
  let previewChars = $state<CharacterGlossaryEntry[]>([]);
  let previewTerms = $state<TermGlossaryEntry[]>([]);
  let hasResults = $state(false);
  let statsSummary = $state<{ updatedChars: number; updatedTerms: number }>({ updatedChars: 0, updatedTerms: 0 });

  // Термины, которые прошли проверку моделью и ждут отбора
  let reviewTerms = $state<(ReviewedTerm & { selected: boolean })[]>([]);
  let reviewNote = $state("");
  let selectedReviewCount = $derived(reviewTerms.filter((t) => t.selected).length);

  onMount(() => {
    try {
      const savedApi = localStorage.getItem(LLM_API_KEY);
      if (savedApi) {
        apiUrl = savedApi.replace("localhost", "127.0.0.1");
      }
      const savedModel = localStorage.getItem(LLM_MODEL_KEY);
      if (savedModel) selectedModel = savedModel;
    } catch {
      // Игнорируем недоступность storage
    }
  });

  $effect(() => {
    if (show) {
      activeTab = initialTab;
      errorMessage = null;
      hasResults = false;
      reviewTerms = [];
      reviewNote = "";
      characterRows = [];
      termRows = [];
      previewChars = JSON.parse(JSON.stringify(characters));
      previewTerms = JSON.parse(JSON.stringify(terms));
    }
  });

  // Вычисляемые счетчики
  let emptyCharsCount = $derived.by(() => {
    return characters.filter(
      (c) => !c.translated_name.trim() || c.gender === "unknown" || !c.gender.trim()
    ).length;
  });

  let emptyTermsCount = $derived.by(() => {
    return terms.filter((t) => !t.translated_term.trim()).length;
  });

  function saveSettings() {
    try {
      localStorage.setItem(LLM_API_KEY, apiUrl.trim());
      localStorage.setItem(LLM_MODEL_KEY, selectedModel.trim());
    } catch {}
  }

  async function testConnection() {
    testingConnection = true;
    connectionStatus = "idle";
    connectionMsg = "";
    errorMessage = null;
    saveSettings();

    try {
      const models = await invoke<string[]>("check_llm_models", { apiUrl: apiUrl.trim() });
      availableModels = models;
      connectionStatus = "success";
      if (models.length > 0) {
        connectionMsg = `Найдено моделей: ${models.length}`;
        if (!models.includes(selectedModel)) {
          selectedModel = models[0];
          saveSettings();
        }
      } else {
        connectionMsg = "Сервер доступен (модели не возвращены)";
      }
    } catch (e) {
      connectionStatus = "error";
      connectionMsg = `Ошибка: ${String(e)}`;
    } finally {
      testingConnection = false;
    }
  }

  function failedBatchesMessage(count: number) {
    return `Часть данных модель не обработала (пакетов с ошибкой: ${count}). Эти записи остались без изменений, запустите обработку ещё раз.`;
  }

  function beginRun(step: string) {
    isProcessing = true;
    errorMessage = null;
    progress = null;
    stopRequested = false;
    hasResults = false;
    finishedRun = null;
    progressStep = step;
    saveSettings();
  }

  /** Канал итогов пакетов: записи появляются в окне сразу, как только их вернула модель. */
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

  async function startCharactersAutofill() {
    beginRun("Сбор реплик персонажей из сценария...");
    const rows = previewChars.flatMap((c, index) =>
      !onlyEmpty || !c.translated_name.trim() || c.gender === "unknown" || !c.gender.trim()
        ? [{ index, samples: [], done: false }]
        : []
    );
    characterRows = rows;

    try {
      const result = await invoke<AutofillResult<CharacterGlossaryEntry>>("autofill_glossary_characters", {
        gameDir,
        apiUrl: apiUrl.trim(),
        model: selectedModel.trim(),
        characters: previewChars,
        onlyEmpty,
        onBatch: batchChannel<CharacterResult>((items) => {
          for (const { samples, ...character } of items) {
            const row = characterRows.find((r) => !r.done && previewChars[r.index].alias === character.alias);
            if (!row) continue;
            previewChars[row.index] = character;
            row.samples = samples;
            row.done = true;
            hasResults = true;
          }
          if (!stopRequested) progressStep = "Модель определяет имена и род персонажей...";
        })
      });
      finishedRun = { noun: "Обработано персонажей", rows: () => characterRows };
      if (result.failed_batches > 0) errorMessage = failedBatchesMessage(result.failed_batches);
    } catch (e) {
      errorMessage = `Не удалось заполнить персонажей: ${String(e)}`;
    } finally {
      isProcessing = false;
    }
  }

  async function startTermsExtraction() {
    beginRun("Поиск названий по всему тексту игры...");
    reviewTerms = [];
    reviewNote = "";

    try {
      const result = await invoke<ExtractedTerms>("extract_glossary_terms_from_game", {
        gameDir,
        apiUrl: apiUrl.trim(),
        model: selectedModel.trim(),
        maxCandidates,
        knownTerms: previewTerms.map((t) => t.source_term),
        onBatch: batchChannel<ReviewedTerm>((items) => {
          reviewTerms.push(...items.map((item) => ({ ...item, selected: true })));
          if (!stopRequested) progressStep = "Модель проверяет кандидатов...";
        })
      });

      reviewNote = result.candidates === 0
        ? "В тексте игры не нашлось новых названий."
        : `Модель оставила ${result.items.length} из ${result.candidates} кандидатов.`;
      if (stopRequested) reviewNote = `Остановлено. Модель оставила ${result.items.length} терминов.`;
      if (result.single_words_skipped) {
        reviewNote += " В языке игры заглавные пишутся у всех существительных, поэтому искались только фразы из нескольких слов.";
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

  function setAllReviewed(selected: boolean) {
    for (const term of reviewTerms) term.selected = selected;
  }

  function addReviewedTerms() {
    const chosen = reviewTerms
      .filter((t) => t.selected)
      .map(({ selected, count, example, ...term }) => term);
    previewTerms = [...previewTerms, ...chosen];
    statsSummary.updatedTerms = chosen.length;
    reviewTerms = [];
    reviewNote = "";
    hasResults = true;
    finishedRun = null;
    progressStep = `Готово! Добавлено терминов: ${chosen.length}`;
  }

  async function startTermsAutofill() {
    beginRun("Перевод и категоризация терминов через LLM...");
    const rows = previewTerms.flatMap((t, index) =>
      !onlyEmpty || !t.translated_term.trim() ? [{ index, samples: [], done: false }] : []
    );
    termRows = rows;

    try {
      const result = await invoke<AutofillResult<TermGlossaryEntry>>("autofill_glossary_terms", {
        apiUrl: apiUrl.trim(),
        model: selectedModel.trim(),
        terms: previewTerms,
        onlyEmpty,
        onBatch: batchChannel<TermGlossaryEntry>((items) => {
          for (const term of items) {
            const row = termRows.find((r) => !r.done && previewTerms[r.index].id === term.id);
            if (!row) continue;
            previewTerms[row.index] = term;
            row.done = true;
            hasResults = true;
          }
        })
      });
      finishedRun = { noun: "Обработано терминов", rows: () => termRows };
      if (result.failed_batches > 0) errorMessage = failedBatchesMessage(result.failed_batches);
    } catch (e) {
      errorMessage = `Не удалось обработать термины: ${String(e)}`;
    } finally {
      isProcessing = false;
    }
  }

  function applyChanges() {
    onapply(previewChars, previewTerms);
    onclose();
  }
</script>

{#if show}
  <div 
    class="modal-backdrop" 
    onclick={(e) => { if (e.target === e.currentTarget && !isProcessing) onclose(); }}
    onkeydown={(e) => { if (e.key === 'Escape' && !isProcessing) onclose(); }}
    role="dialog"
    aria-modal="true"
    aria-label="ИИ-автозаполнение глоссария"
    tabindex="-1"
  >
    <div class="ai-modal-box">
      <!-- Заголовок -->
      <div class="modal-header">
        <div class="header-title-wrap">
          <div class="header-icon-badge">
            <Sparkles size={18} class="text-violet-400" />
          </div>
          <div>
            <h3>ИИ-Автозаполнение глоссария</h3>
            <p class="subtitle">Нейросетевой перевод имён, определение рода (М/Ж/Ср) и извлечение терминов</p>
          </div>
        </div>
        <button class="btn-icon-close" onclick={onclose} disabled={isProcessing} aria-label="Закрыть">
          <X size={16} />
        </button>
      </div>

      <!-- Блок настроек подключения к LLM -->
      <div class="config-card">
        <div class="config-header">
          <div class="flex items-center gap-2">
            <Server size={14} class="text-slate-400" />
            <span class="text-xs font-semibold text-slate-300 uppercase tracking-wider">Подключение к LLM (LM Studio / Ollama)</span>
          </div>
          <button 
            class="btn-test-connection" 
            onclick={testConnection} 
            disabled={testingConnection || isProcessing}
          >
            {#if testingConnection}
              <Loader2 size={13} class="spin" />
              <span>Проверка...</span>
            {:else}
              <RefreshCw size={13} />
              <span>Проверить сервер</span>
            {/if}
          </button>
        </div>

        <div class="config-grid">
          <div class="field-wrap">
            <label for="ai-endpoint-url" class="field-label">API Endpoint URL</label>
            <input 
              id="ai-endpoint-url"
              type="text" 
              bind:value={apiUrl} 
              placeholder="http://localhost:1234/v1" 
              class="input-compact font-mono text-xs"
              disabled={isProcessing}
            />
          </div>

          <div class="field-wrap">
            <label for="ai-model-select" class="field-label">Модель перевода</label>
            {#if availableModels.length > 0}
              <select 
                id="ai-model-select"
                bind:value={selectedModel} 
                class="input-compact text-xs cursor-pointer"
                disabled={isProcessing}
              >
                {#each availableModels as m}
                  <option value={m}>{m}</option>
                {/each}
              </select>
            {:else}
              <input 
                id="ai-model-select"
                type="text" 
                bind:value={selectedModel} 
                placeholder="mistral-small, qwen2.5, ..." 
                class="input-compact text-xs"
                disabled={isProcessing}
              />
            {/if}
          </div>
        </div>

        {#if connectionStatus !== 'idle'}
          <div class="connection-status-pill" class:success={connectionStatus === 'success'} class:error={connectionStatus === 'error'}>
            {#if connectionStatus === 'success'}
              <Check size={13} class="text-emerald-400" />
              <span>{connectionMsg}</span>
            {:else}
              <AlertCircle size={13} class="text-rose-400" />
              <span class="truncate">{connectionMsg}</span>
            {/if}
          </div>
        {/if}
      </div>

      <!-- Вкладки режимов: Персонажи / Термины -->
      <div class="ai-tabs">
        <button 
          class="ai-tab" 
          class:active={activeTab === 'characters'} 
          onclick={() => activeTab = 'characters'}
          disabled={isProcessing}
        >
          <Users size={14} />
          <span>Персонажи</span>
          <span class="tab-badge">{characters.length} ({emptyCharsCount} не заполнено)</span>
        </button>
        <button 
          class="ai-tab" 
          class:active={activeTab === 'terms'} 
          onclick={() => activeTab = 'terms'}
          disabled={isProcessing}
        >
          <BookMarked size={14} />
          <span>Термины и локации</span>
          <span class="tab-badge">{terms.length} ({emptyTermsCount} не заполнено)</span>
        </button>
      </div>

      <!-- Тело активного режима -->
      <div class="ai-body">
        <!-- Прогресс / Ошибка -->
        {#if isProcessing}
          <div class="progress-box">
            <div class="progress-line">
              <Loader2 size={18} class="spin text-violet-400" />
              <span class="progress-text">{progressStep}</span>
              {#if progress}
                <span class="progress-count">Пакет {progress.batch} из {progress.total}</span>
              {/if}
              <button class="btn-stop" onclick={stopRun} disabled={stopRequested}>
                <Square size={11} />
                <span>Остановить</span>
              </button>
            </div>
            {#if progress}
              <div class="progress-track">
                <div class="progress-fill" style={`width: ${(progress.batch / progress.total) * 100}%`}></div>
              </div>
            {/if}
          </div>
        {/if}

        {#if errorMessage}
          <div class="error-box">
            <AlertCircle size={16} class="text-rose-400 shrink-0" />
            <span class="text-xs text-rose-200">{errorMessage}</span>
          </div>
        {/if}

        {#if hasResults && !isProcessing}
          <div class="result-banner">
            <Check size={18} class="text-emerald-400 shrink-0" />
            <div class="flex-1 text-xs">
              <span class="font-semibold text-emerald-300">{resultLabel}</span>
              <p class="text-slate-400 mt-0.5">Нажмите «Применить к глоссарию», чтобы перенести результаты в таблицу проекта.</p>
            </div>
            <button class="btn-apply-results" onclick={applyChanges}>
              <Check size={14} />
              <span>Применить к глоссарию</span>
            </button>
          </div>
        {/if}
        {#if activeTab === 'characters'}
          <div class="tab-content">
            {#if characterRows.length === 0}
            <div class="info-banner">
              <Wand2 size={16} class="text-violet-400 shrink-0 mt-0.5" />
              <div class="text-xs text-slate-300 space-y-1">
                <p><strong>Автоперевод имён и определение рода:</strong></p>
                <p>Нейросеть транскрибирует имена на русский язык, определяет грамматический род (М/Ж/Ср) для согласования глаголов, а также собирает контекстные фразы персонажей из сценария игры.</p>
              </div>
            </div>
            {/if}

            <div class="options-group">
              <label class="checkbox-label">
                <input type="checkbox" bind:checked={onlyEmpty} disabled={isProcessing} />
                <span>Заполнять только непереведенные имена и пустой род (рекомендуется)</span>
              </label>
            </div>

            {#if characterRows.length === 0}
            <div class="stats-overview">
              <div class="stat-card">
                <span class="stat-num">{characters.length}</span>
                <span class="stat-desc">Всего персонажей</span>
              </div>
              <div class="stat-card stat-highlight">
                <span class="stat-num text-amber-400">{emptyCharsCount}</span>
                <span class="stat-desc">Требуют заполнения</span>
              </div>
              <div class="stat-card">
                <span class="stat-num text-emerald-400">{characters.length - emptyCharsCount}</span>
                <span class="stat-desc">Уже готово</span>
              </div>
            </div>
            {/if}

            <!-- Кнопка запуска -->
            <button 
              class="btn-run-generation" 
              onclick={startCharactersAutofill} 
              disabled={isProcessing || characters.length === 0}
            >
              {#if isProcessing}
                <Loader2 size={16} class="spin" />
                <span>Генерация перевода имён...</span>
              {:else}
                <Sparkles size={16} />
                <span>Запустить автозаполнение персонажей через ИИ</span>
              {/if}
            </button>

            {#if characterRows.length > 0}
              <div class="review-panel">
                {#each characterRows as row (row.index)}
                  {@const character = previewChars[row.index]}
                  <div class="result-item" class:result-waiting={!row.done}>
                    <div class="result-title">
                      <span class="review-term">{character.original_name || character.alias}</span>
                      <span class="review-count">{character.alias}</span>
                      {#if !row.done}
                        <span class="review-count">{isProcessing ? "в очереди" : "не обработан"}</span>
                      {/if}
                    </div>
                    {#if row.done}
                      <div class="result-fields">
                        <input
                          type="text"
                          class="input-compact text-xs"
                          bind:value={character.translated_name}
                          aria-label={`Перевод имени ${character.alias}`}
                        />
                        <select
                          class="input-compact text-xs cursor-pointer"
                          bind:value={character.gender}
                          aria-label={`Род персонажа ${character.alias}`}
                        >
                          <option value="female">Женский</option>
                          <option value="male">Мужской</option>
                          <option value="neuter">Средний</option>
                          <option value="unknown">Не указан</option>
                        </select>
                        <input
                          type="text"
                          class="input-compact text-xs"
                          bind:value={character.notes}
                          placeholder="Заметка"
                          aria-label={`Заметка о персонаже ${character.alias}`}
                        />
                      </div>
                      {#if row.samples.length > 0}
                        <ul class="result-samples">
                          {#each row.samples as sample}
                            <li>{sample}</li>
                          {/each}
                        </ul>
                      {:else}
                        <p class="review-note">Реплик не нашлось: модель судила только по имени.</p>
                      {/if}
                    {/if}
                  </div>
                {/each}
              </div>
            {/if}
          </div>
        {:else}
          <div class="tab-content">
            {#if termRows.length === 0 && reviewTerms.length === 0}
            <div class="info-banner">
              <BookMarked size={16} class="text-emerald-400 shrink-0 mt-0.5" />
              <div class="text-xs text-slate-300 space-y-1">
                <p><strong>Словарь терминов, локаций и предметов:</strong></p>
                <p>Нейросеть переведёт термины и определит категории (локация, титул, предмет). Новые названия ищутся по всему тексту игры: модель отсеивает лишнее, а вы выбираете, что добавить в глоссарий.</p>
              </div>
            </div>
            {/if}

            <div class="options-group space-y-2">
              <label class="checkbox-label">
                <input type="checkbox" bind:checked={extractTermsFromGame} disabled={isProcessing} />
                <span>Найти новые термины в тексте игры</span>
              </label>

              {#if extractTermsFromGame}
                <div class="ml-6 flex items-center gap-2 text-xs text-slate-300">
                  <label for="ai-max-candidates">Кандидатов на проверку:</label>
                  <input
                    id="ai-max-candidates"
                    type="number"
                    min="20"
                    max="200"
                    step="20"
                    bind:value={maxCandidates}
                    class="input-compact w-20 text-center text-xs"
                    disabled={isProcessing}
                  />
                  <span class="review-note">каждые 20 это один запрос к модели</span>
                </div>
              {:else}
                <label class="checkbox-label">
                  <input type="checkbox" bind:checked={onlyEmpty} disabled={isProcessing} />
                  <span>Переводить только непереведенные термины</span>
                </label>
              {/if}
            </div>

            <!-- Кнопка запуска терминов -->
            <button
              class="btn-run-generation btn-run-terms"
              onclick={extractTermsFromGame ? startTermsExtraction : startTermsAutofill}
              disabled={isProcessing}
            >
              {#if isProcessing}
                <Loader2 size={16} class="spin" />
                <span>Обработка терминов через LLM...</span>
              {:else}
                <Wand2 size={16} />
                <span>{extractTermsFromGame ? 'Найти термины в игре через ИИ' : 'Перевести термины через ИИ'}</span>
              {/if}
            </button>

            {#if termRows.length > 0}
              <div class="review-panel">
                {#each termRows as row (row.index)}
                  {@const term = previewTerms[row.index]}
                  <div class="result-item" class:result-waiting={!row.done}>
                    <div class="result-title">
                      <span class="review-term">{term.source_term}</span>
                      {#if !row.done}
                        <span class="review-count">{isProcessing ? "в очереди" : "не обработан"}</span>
                      {/if}
                    </div>
                    {#if row.done}
                      <div class="result-fields">
                        <input
                          type="text"
                          class="input-compact text-xs"
                          bind:value={term.translated_term}
                          aria-label={`Перевод термина ${term.source_term}`}
                        />
                        <select
                          class="input-compact text-xs cursor-pointer"
                          bind:value={term.category}
                          aria-label={`Категория термина ${term.source_term}`}
                        >
                          <option value="general">Общее</option>
                          <option value="location">Локация</option>
                          <option value="title">Титул</option>
                          <option value="item">Предмет</option>
                        </select>
                        <input
                          type="text"
                          class="input-compact text-xs"
                          bind:value={term.notes}
                          placeholder="Заметка"
                          aria-label={`Заметка о термине ${term.source_term}`}
                        />
                      </div>
                    {/if}
                  </div>
                {/each}
              </div>
            {/if}

            {#if reviewNote || reviewTerms.length > 0}
              <div class="review-panel">
                <div class="review-header">
                  <span class="review-note">{reviewNote || "Найденные термины появляются здесь по мере проверки."}</span>
                  {#if reviewTerms.length > 0}
                    <div class="review-toggles">
                      <button class="btn-link" onclick={() => setAllReviewed(true)}>Выбрать все</button>
                      <button class="btn-link" onclick={() => setAllReviewed(false)}>Снять все</button>
                    </div>
                  {/if}
                </div>

                {#each reviewTerms as term (term.id)}
                  <div class="review-row" class:review-row-off={!term.selected}>
                    <input
                      type="checkbox"
                      class="review-check"
                      bind:checked={term.selected}
                      aria-label={`Добавить термин ${term.source_term}`}
                    />
                    <div class="review-source">
                      <span class="review-term">{term.source_term} <span class="review-count">× {term.count}</span></span>
                      <span class="review-example" title={term.example}>{term.example}</span>
                    </div>
                    <input
                      type="text"
                      class="input-compact text-xs"
                      bind:value={term.translated_term}
                      aria-label={`Перевод термина ${term.source_term}`}
                    />
                    <select
                      class="input-compact text-xs cursor-pointer"
                      bind:value={term.category}
                      aria-label={`Категория термина ${term.source_term}`}
                    >
                      <option value="general">Общее</option>
                      <option value="location">Локация</option>
                      <option value="title">Титул</option>
                      <option value="item">Предмет</option>
                    </select>
                  </div>
                {/each}

                {#if reviewTerms.length > 0}
                  <button class="btn-apply-results review-add" onclick={addReviewedTerms} disabled={selectedReviewCount === 0 || isProcessing}>
                    <Check size={14} />
                    <span>Добавить выбранные ({selectedReviewCount})</span>
                  </button>
                {/if}
              </div>
            {/if}
          </div>
        {/if}

      </div>

      <!-- Футер -->
      <div class="modal-footer">
        <button class="btn-secondary" onclick={onclose} disabled={isProcessing}>
          Закрыть
        </button>
        {#if hasResults && !isProcessing}
          <button class="btn-primary" onclick={applyChanges}>
            <Check size={14} />
            <span>Применить и закрыть</span>
          </button>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .modal-backdrop {
    position: fixed;
    inset: 0;
    z-index: 9999;
    background: rgba(10, 14, 23, 0.78);
    backdrop-filter: blur(6px);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 1.5rem;
  }

  .ai-modal-box {
    background: #0f172a;
    border: 1px solid rgba(139, 92, 246, 0.28);
    border-radius: 14px;
    box-shadow: 0 25px 60px -15px rgba(0, 0, 0, 0.75), 0 0 35px rgba(139, 92, 246, 0.12);
    width: 100%;
    max-width: 760px;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    animation: modal-pop 0.2s cubic-bezier(0.16, 1, 0.3, 1);
  }

  @keyframes modal-pop {
    from {
      opacity: 0;
      transform: scale(0.96) translateY(8px);
    }
    to {
      opacity: 1;
      transform: scale(1) translateY(0);
    }
  }

  .modal-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 1.1rem 1.4rem;
    border-bottom: 1px solid rgba(51, 65, 85, 0.5);
    background: rgba(15, 23, 42, 0.95);
  }

  .header-title-wrap {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }

  .header-icon-badge {
    width: 36px;
    height: 36px;
    border-radius: 9px;
    background: rgba(139, 92, 246, 0.15);
    border: 1px solid rgba(139, 92, 246, 0.35);
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .modal-header h3 {
    margin: 0;
    font-size: 1rem;
    font-weight: 600;
    color: #f8fafc;
  }

  .modal-header .subtitle {
    margin: 0;
    font-size: 0.73rem;
    color: #94a3b8;
  }

  .btn-icon-close {
    background: transparent;
    border: none;
    color: #94a3b8;
    cursor: pointer;
    padding: 0.4rem;
    border-radius: 6px;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: all 0.15s;
  }

  .btn-icon-close:hover {
    color: #f8fafc;
    background: rgba(255, 255, 255, 0.08);
  }

  .config-card {
    padding: 1rem 1.4rem;
    background: rgba(30, 41, 59, 0.45);
    border-bottom: 1px solid rgba(51, 65, 85, 0.4);
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .config-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .btn-test-connection {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    background: rgba(51, 65, 85, 0.6);
    border: 1px solid rgba(71, 85, 105, 0.6);
    color: #cbd5e1;
    font-size: 0.72rem;
    padding: 0.28rem 0.65rem;
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.15s;
  }

  .btn-test-connection:hover:not(:disabled) {
    background: rgba(71, 85, 105, 0.8);
    color: #f8fafc;
    border-color: rgba(148, 163, 184, 0.5);
  }

  .config-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 0.75rem;
  }

  .field-wrap {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }

  .field-label {
    font-size: 0.7rem;
    font-weight: 500;
    color: #94a3b8;
  }

  .input-compact {
    background: #090d16;
    border: 1px solid rgba(51, 65, 85, 0.8);
    color: #f1f5f9;
    padding: 0.38rem 0.65rem;
    border-radius: 6px;
    outline: none;
    transition: border-color 0.15s;
  }

  .input-compact:focus {
    border-color: rgba(139, 92, 246, 0.6);
  }

  .connection-status-pill {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.72rem;
    padding: 0.3rem 0.6rem;
    border-radius: 6px;
  }

  .connection-status-pill.success {
    background: rgba(16, 185, 129, 0.1);
    border: 1px solid rgba(16, 185, 129, 0.25);
    color: #34d399;
  }

  .connection-status-pill.error {
    background: rgba(244, 63, 94, 0.1);
    border: 1px solid rgba(244, 63, 94, 0.25);
    color: #fb7185;
  }

  .ai-tabs {
    display: flex;
    border-bottom: 1px solid rgba(51, 65, 85, 0.5);
    background: rgba(15, 23, 42, 0.5);
  }

  .ai-tab {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.45rem;
    padding: 0.65rem 1rem;
    background: transparent;
    border: none;
    border-bottom: 2px solid transparent;
    color: #94a3b8;
    font-size: 0.78rem;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.15s;
  }

  .ai-tab:hover {
    color: #f1f5f9;
    background: rgba(255, 255, 255, 0.02);
  }

  .ai-tab.active {
    color: #c084fc;
    border-bottom-color: #a855f7;
    background: rgba(168, 85, 247, 0.06);
  }

  .tab-badge {
    font-size: 0.68rem;
    background: rgba(51, 65, 85, 0.6);
    color: #cbd5e1;
    padding: 0.12rem 0.45rem;
    border-radius: 10px;
  }

  .ai-body {
    padding: 1.2rem 1.4rem;
    display: flex;
    flex-direction: column;
    gap: 1rem;
    overflow-y: auto;
    max-height: 55vh;
  }

  .tab-content {
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }

  .info-banner {
    display: flex;
    gap: 0.65rem;
    background: rgba(30, 41, 59, 0.5);
    border: 1px solid rgba(51, 65, 85, 0.5);
    padding: 0.75rem 0.9rem;
    border-radius: 8px;
  }

  .options-group {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .checkbox-label {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.78rem;
    color: #cbd5e1;
    cursor: pointer;
    user-select: none;
  }

  .checkbox-label input[type="checkbox"] {
    accent-color: #a855f7;
    width: 15px;
    height: 15px;
    cursor: pointer;
  }

  .stats-overview {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 0.65rem;
  }

  .stat-card {
    background: rgba(15, 23, 42, 0.6);
    border: 1px solid rgba(51, 65, 85, 0.45);
    border-radius: 8px;
    padding: 0.6rem 0.75rem;
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
  }

  .stat-card.stat-highlight {
    border-color: rgba(245, 158, 11, 0.35);
    background: rgba(245, 158, 11, 0.05);
  }

  .stat-num {
    font-size: 1.15rem;
    font-weight: 700;
    color: #f1f5f9;
  }

  .stat-desc {
    font-size: 0.68rem;
    color: #94a3b8;
  }

  .btn-run-generation {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.5rem;
    background: linear-gradient(135deg, #7c3aed, #9333ea);
    border: 1px solid rgba(192, 132, 252, 0.4);
    color: #ffffff;
    font-size: 0.82rem;
    font-weight: 600;
    padding: 0.7rem 1.2rem;
    border-radius: 8px;
    cursor: pointer;
    transition: all 0.2s;
    box-shadow: 0 4px 14px rgba(124, 58, 237, 0.35);
  }

  .btn-run-generation:hover:not(:disabled) {
    background: linear-gradient(135deg, #6d28d9, #7e22ce);
    box-shadow: 0 6px 18px rgba(124, 58, 237, 0.5);
    transform: translateY(-1px);
  }

  .btn-run-terms {
    background: linear-gradient(135deg, #059669, #0d9488);
    border-color: rgba(52, 211, 153, 0.4);
    box-shadow: 0 4px 14px rgba(5, 150, 105, 0.35);
  }

  .btn-run-terms:hover:not(:disabled) {
    background: linear-gradient(135deg, #047857, #0f766e);
    box-shadow: 0 6px 18px rgba(5, 150, 105, 0.5);
  }

  .btn-run-generation:disabled {
    opacity: 0.5;
    cursor: not-allowed;
    transform: none;
    box-shadow: none;
  }

  .progress-box {
    position: sticky;
    top: 0;
    z-index: 1;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    background: #1b1a3a;
    border: 1px solid rgba(139, 92, 246, 0.25);
    padding: 0.75rem 1rem;
    border-radius: 8px;
  }

  .progress-line {
    display: flex;
    align-items: center;
    gap: 0.65rem;
  }

  .progress-text {
    flex: 1;
    min-width: 0;
    font-size: 0.75rem;
    color: #e2e8f0;
  }

  .progress-count {
    font-size: 0.72rem;
    color: #94a3b8;
    white-space: nowrap;
  }

  .progress-track {
    height: 4px;
    border-radius: 2px;
    background: rgba(51, 65, 85, 0.7);
    overflow: hidden;
  }

  .progress-fill {
    height: 100%;
    background: #a855f7;
    transition: width 0.3s;
  }

  .btn-stop {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    background: rgba(51, 65, 85, 0.6);
    border: 1px solid rgba(71, 85, 105, 0.6);
    color: #cbd5e1;
    font-size: 0.72rem;
    padding: 0.28rem 0.65rem;
    border-radius: 6px;
    cursor: pointer;
    white-space: nowrap;
  }

  .btn-stop:hover:not(:disabled) {
    color: #f8fafc;
    border-color: rgba(148, 163, 184, 0.5);
  }

  .btn-stop:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .error-box {
    display: flex;
    align-items: center;
    gap: 0.65rem;
    background: rgba(244, 63, 94, 0.1);
    border: 1px solid rgba(244, 63, 94, 0.3);
    padding: 0.75rem 1rem;
    border-radius: 8px;
  }

  .result-banner {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    background: rgba(16, 185, 129, 0.08);
    border: 1px solid rgba(16, 185, 129, 0.3);
    padding: 0.75rem 1rem;
    border-radius: 8px;
  }

  .btn-apply-results {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    background: #059669;
    border: 1px solid rgba(52, 211, 153, 0.5);
    color: #fff;
    font-size: 0.75rem;
    font-weight: 600;
    padding: 0.45rem 0.85rem;
    border-radius: 6px;
    cursor: pointer;
    white-space: nowrap;
    transition: all 0.15s;
  }

  .btn-apply-results:hover:not(:disabled) {
    background: #047857;
  }

  .btn-apply-results:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .review-panel {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    background: rgba(15, 23, 42, 0.6);
    border: 1px solid rgba(51, 65, 85, 0.5);
    border-radius: 8px;
    padding: 0.75rem;
  }

  .review-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    margin-bottom: 0.25rem;
  }

  .review-note {
    font-size: 0.72rem;
    color: #94a3b8;
  }

  .review-toggles {
    display: flex;
    gap: 0.6rem;
    flex-shrink: 0;
  }

  .btn-link {
    background: transparent;
    border: none;
    padding: 0;
    color: #c084fc;
    font-size: 0.72rem;
    cursor: pointer;
  }

  .btn-link:hover {
    color: #e9d5ff;
  }

  .review-row {
    display: grid;
    grid-template-columns: auto minmax(0, 1.2fr) minmax(0, 1fr) 6.2rem;
    align-items: center;
    gap: 0.5rem;
  }

  .review-row-off {
    opacity: 0.5;
  }

  .review-check {
    accent-color: #a855f7;
    width: 15px;
    height: 15px;
    cursor: pointer;
  }

  .review-source {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .review-term {
    font-size: 0.78rem;
    color: #f1f5f9;
  }

  .review-count {
    font-size: 0.68rem;
    color: #94a3b8;
  }

  .review-example {
    font-size: 0.68rem;
    color: #94a3b8;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .result-item {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    padding: 0.5rem 0;
    border-top: 1px solid rgba(51, 65, 85, 0.35);
  }

  .result-item:first-child {
    border-top: none;
    padding-top: 0;
  }

  .result-waiting {
    opacity: 0.55;
  }

  .result-title {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    min-width: 0;
  }

  .result-fields {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 7rem minmax(0, 1.4fr);
    gap: 0.4rem;
  }

  .result-samples {
    margin: 0;
    padding-left: 1rem;
    font-size: 0.72rem;
    color: #94a3b8;
  }

  .review-add {
    align-self: flex-end;
    margin-top: 0.35rem;
  }

  .modal-footer {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 0.65rem;
    padding: 0.9rem 1.4rem;
    border-top: 1px solid rgba(51, 65, 85, 0.5);
    background: rgba(15, 23, 42, 0.95);
  }

  .btn-secondary {
    background: rgba(51, 65, 85, 0.5);
    border: 1px solid rgba(71, 85, 105, 0.6);
    color: #cbd5e1;
    font-size: 0.78rem;
    padding: 0.45rem 0.95rem;
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.15s;
  }

  .btn-secondary:hover:not(:disabled) {
    background: rgba(71, 85, 105, 0.7);
    color: #f8fafc;
  }

  .btn-primary {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    background: #7c3aed;
    border: 1px solid rgba(168, 85, 247, 0.5);
    color: #ffffff;
    font-size: 0.78rem;
    font-weight: 600;
    padding: 0.45rem 1rem;
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.15s;
  }

  .btn-primary:hover {
    background: #6d28d9;
  }

  :global(.spin) {
    animation: spin 1s linear infinite;
  }

  @keyframes spin {
    from { transform: rotate(0deg); }
    to { transform: rotate(360deg); }
  }
</style>
