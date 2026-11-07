<script lang="ts">
  import { onDestroy, tick, type Snippet } from "svelte";
  import { Channel, invoke } from "@tauri-apps/api/core";
  import { AlertCircle, Check, ChevronDown, ChevronRight, ClipboardCopy, ClipboardPaste, Loader2, Sparkles, Square } from "lucide-svelte";
  import EntryPicker from "./EntryPicker.svelte";
  import LlmChoice from "./LlmChoice.svelte";
  import { baseNames, characterTitle, chatNames, currentNameOrigins, missingNameAnalysis, savedNameAnalysis, formNames, formTranslation, fullName, nameTargetKey, translatedTitle, type CanonicalName, type CharacterNameOrigin, type NameBatch, type NameField, type NameForm, type NameTarget } from "./name-forms";
  import { beginLlmRun, endLlmRun, llmRun, setLlmProgress, type LlmRequestProgress } from "./llm-run.svelte";
  import LlmProgress from "./LlmProgress.svelte";
  import { nameOptions, nameCheckNotice, modelNameChoice, type NameVariant } from "./name-options";
  import { LLM_ACTIONS, outsideNote, targetFor, type LlmAction, type LlmActionSettings, type LlmSettings } from "./llm-settings";
  import { nameRules, loadNameRules, setNameRule, missingNameRuleLanguages, normalizeLanguageName, type LanguageRule } from "./name-rules.svelte";

  interface PanelCharacter extends CanonicalName {
    alias: string;
    gender: string;
    merged: { alias: string; original_name: string }[];
    forms: NameForm[];
    name_origins?: CharacterNameOrigin[];
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
    errors?: { name: string; reason: string }[];
  }

  interface BatchProgress<T> {
    batch: number;
    total_batches: number;
    failed_batches: number;
    items: T[];
  }

  interface NameSuggestion {
    id: string;
    translated: string;
    origin: string;
    issues: string[];
    variants?: NameVariant[];
    glossary_variants?: NameVariant[];
  }

  interface NameOrigin {
    id: string;
    name: string;
    context: string;
    languages: string[];
    reason: string;
  }


  interface TokenSuggestion {
    name: string;
    value: string;
    note: string;
  }

  interface PanelSpeaker {
    name: string;
    lines: number;
    translated: string;
    gender: string;
  }

  interface SpeakerSuggestion {
    name: string;
    translated: string;
    gender: string;
  }

  type GlossaryTab = "characters" | "terms" | "tokens" | "scenes" | "speakers";

  let {
    tab,
    gameDir,
    characters,
    terms,
    tokens,
    speakers,
    hints,
    settings,
    descriptionsOpen = $bindable(false),
    descriptions,
    onselect,
    onnames,
    onorigins,
    onterms,
    ontokens,
    onspeakers
  }: {
    tab: GlossaryTab;
    gameDir: string;
    characters: PanelCharacter[];
    terms: PanelTerm[];
    tokens: PanelToken[];
    speakers: PanelSpeaker[];
    hints: object;
    settings: LlmSettings;
    descriptionsOpen?: boolean;
    descriptions: Snippet;
    onselect: (action: LlmAction, choice: LlmActionSettings) => void;
    onnames: (rows: { alias: string; field: NameField; source: string; translated: string }[]) => void;
    onorigins: (rows: { alias: string; origin: CharacterNameOrigin }[], overwrite?: boolean) => void;
    onterms: (rows: { id: string; translated: string; category: string; notes: string }[]) => void;
    ontokens: (rows: { name: string; value: string; note: string }[]) => void;
    onspeakers: (rows: { name: string; translated: string; gender: string }[]) => void;
  } = $props();

  const SECTIONS: Record<GlossaryTab, LlmAction[]> = {
    characters: ["descriptions", "names"],
    terms: ["terms", "descriptions"],
    tokens: ["tokens"],
    scenes: ["descriptions"],
    speakers: ["speakers"]
  };

  let panelOpen = $state(false);
  let sectionByTab = $state<Record<GlossaryTab, LlmAction>>({ characters: "descriptions", terms: "terms", tokens: "tokens", scenes: "descriptions", speakers: "speakers" });
  let openSection = $derived(sectionByTab[tab]);
  let openOutsideNote = $derived(outsideNote(settings, openSection));

  // Запуск этой панели. Настройки и кнопки отключает любой запуск (`llmRun.active`)
  let isProcessing = $state(false);
  let progressStep = $state("");
  let progress = $state<{ batch: number; total: number } | null>(null);
  let stopRequested = $state(false);
  let errorMessage = $state<string | null>(null);
  let doneMessage = $state("");
  let progressChannelSequence = 0;

  function requestProgressChannel(transform = (message: LlmRequestProgress) => message) {
    const sequence = ++progressChannelSequence;
    const channel = new Channel<LlmRequestProgress>();
    channel.onmessage = (message) => {
      if (isProcessing && sequence === progressChannelSequence) setLlmProgress(transform(message));
    };
    return channel;
  }

  $effect(() => { descriptionsOpen = panelOpen && openSection === "descriptions"; });
  // Сообщение относится к разделу, в котором появилось
  $effect(() => {
    void openSection;
    errorMessage = null;
    doneMessage = "";
  });

  // Панель закрывается вместе с глоссарием: результат запроса показать будет негде
  onDestroy(() => {
    if (!isProcessing) return;
    stopRequested = true;
    void invoke("cancel_llm_run");
  });

  function beginRun(step: string) {
    if (!beginLlmRun(step)) return false;
    isProcessing = true;
    errorMessage = null;
    doneMessage = "";
    progress = null;
    progressChannelSequence += 1;
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

  // Обмен с чат-ботом вручную, без API. Состояние живёт здесь: разделы меняются, пока пользователь в чате
  type ChatSection = "names" | "terms";
  let chatRequests = $state<Record<ChatSection, string>>({ names: "", terms: "" });
  let chatAnswers = $state<Record<ChatSection, string>>({ names: "", terms: "" });

  /** Кладёт запрос в буфер обмена. Поле ответа появляется, только когда запрос удалось скопировать. */
  async function copyChatRequest(section: ChatSection, request: Promise<string>) {
    errorMessage = null;
    doneMessage = "";
    try {
      const text = await request;
      await navigator.clipboard.writeText(text);
      // Повторное копирование того же запроса не стирает вставленный ответ
      if (text !== chatRequests[section]) chatAnswers[section] = "";
      chatRequests[section] = text;
      return true;
    } catch (e) {
      errorMessage = `Не удалось скопировать запрос: ${String(e)}`;
      return false;
    }
  }

  function clearChat(section: ChatSection) {
    chatRequests[section] = "";
    chatAnswers[section] = "";
  }

  // Имена
  let unselectedAliases = $state<string[]>([]);
  let nameTargets = $derived(characters.filter((c) => !unselectedAliases.includes(c.alias)));
  let reviewNames = $state<(NameTarget & { key: string; translated: string; origin: string; options: NameVariant[]; notice: string; recommended: string })[]>([]);
  let nameOrigins = $state<NameOrigin[]>([]);
  let analysisSignature = $state("");
  let ruleDrafts = $derived(Object.fromEntries(nameRules.items.map((rule) => [rule.language, rule.rules])));
  let nameSignature = $derived(JSON.stringify({ gameDir, characters: nameTargets.map((c) => ({ alias: c.alias, gender: c.gender, names: [c.first_name, c.last_name, c.translated_first_name, c.translated_last_name], forms: c.forms, merged: c.merged })), cast: gameCast() }));
  let analysisReady = $derived(analysisSignature === nameSignature && nameOrigins.length > 0);
  let originLanguages = $derived([...new Set(nameOrigins.flatMap((o) => o.languages).map(normalizeLanguageName))].filter((language) => language !== "описание" && language !== "неизвестно"));
  let missingRules = $derived(originLanguages.filter((language) => !ruleDrafts[language]?.trim()));

  // Готовый анализ живёт в карточках: закрытие глоссария не требует повторного запуска модели.
  $effect(() => {
    if (isProcessing) return;
    const saved = savedNameAnalysis(nameTargets);
    nameOrigins = saved.origins;
    analysisSignature = saved.complete ? nameSignature : "";
  });

  async function analyzeNames(selected: PanelCharacter[], overwrite = false) {
    const batch = overwrite ? chatNames(selected) : missingNameAnalysis(selected);
    const requests = batch.requests;
    analysisSignature = "";
    nameOrigins = savedNameAnalysis(selected).origins;
    const cast = gameCast();
    const target = targetFor(settings, "names");
    const failures: string[] = [];
    const attemptedLanguages = new Set<string>();
    let generated = 0;
    const generateMissingRules = async (origins: { languages: string[] }[]) => {
      const missing = missingNameRuleLanguages(origins).filter((language) => !attemptedLanguages.has(language));
      progressChannelSequence += 1;
      for (const [index, language] of missing.entries()) {
        if (stopRequested) break;
        attemptedLanguages.add(language);
        progressStep = `2. Генерируем правила: ${language}...`;
        llmRun.note = progressStep;
        setLlmProgress({ phase: "rules", batch: index + 1, total_batches: missing.length, completed: index, total: missing.length, attempt: 1, names: [language] });
        try {
          const rule = await invoke<LanguageRule>("generate_name_rules", { target, language });
          if (!stopRequested && !nameRules.items.some((own) => own.language === language && own.rules.trim())) {
            setNameRule(language, rule.rules); generated += 1;
          }
        } catch (e) { failures.push(`${language}: ${String(e)}`); }
      }
    };
    try {
      await loadNameRules();
      await generateMissingRules(selected.flatMap((char) => currentNameOrigins(char)));
      // Между пакетами анализа освобождается модель для правил только что найденных языков.
      const batchSize = 12;
      for (let offset = 0; offset < requests.length && !stopRequested; offset += batchSize) {
        progressStep = "1. Определяем происхождение имён...";
        llmRun.note = progressStep;
        const chunk = requests.slice(offset, offset + batchSize);
        const origins = await invoke<NameOrigin[]>("analyze_character_names", {
          target, names: chunk, cast, knownLanguages: nameRules.items.filter((rule) => rule.rules.trim()).map((rule) => rule.language),
          onProgress: requestProgressChannel((message) => ({ ...message, batch: Math.floor(offset / batchSize) + message.batch, total_batches: Math.ceil(requests.length / batchSize), completed: offset + message.completed, total: requests.length }))
        });
        if (stopRequested) break;
        nameOrigins.push(...origins);
        onorigins(origins.flatMap((origin) => (batch.targets.get(origin.id) ?? []).map((target) => ({
          alias: target.alias,
          origin: { field: target.field, source: target.source, context: fullName(selected.find((c) => c.alias === target.alias)!), languages: [...origin.languages], reason: origin.reason }
        }))), overwrite);
        await generateMissingRules(origins);
        if (origins.length !== chunk.length) break;
      }
    } catch (e) {
      errorMessage = `Не удалось проанализировать имена: ${String(e)}`;
    } finally {
      if (generated > 0) doneMessage = `Подготовлены правила для новых языков: ${generated}.`;
      else if (!stopRequested && failures.length === 0 && !errorMessage) doneMessage = requests.length > 0 ? "Анализ готов. Можно переключить модель и запустить перевод." : "Происхождения и правила уже заполнены. Изменений нет.";
      if (failures.length > 0) errorMessage = [errorMessage, `Не удалось создать часть правил: ${failures.join("; ")}`].filter(Boolean).join(" ");
    }
  }

  async function startNameAnalysis() {
    await runNameAnalysis($state.snapshot(nameTargets));
  }

  async function runNameAnalysis(selected: PanelCharacter[], overwrite = false) {
    if (selected.length === 0 || !beginRun("1. Определяем происхождение имён...")) return;
    reviewNames = [];
    unchangedNames = 0;
    namesRequested = 0;
    namesFinished = false;
    try {
      await analyzeNames(selected, overwrite);
    } finally { endRun(); }
  }

  async function startNameTranslation() {
    if (!analysisReady || missingRules.length > 0 || !beginRun("Переводим и проверяем имена...")) return;
    try { await startNames(); } finally { endRun(); }
  }

  export async function analyzeCharacter(alias: string) {
    const char = characters.find((char) => char.alias === alias);
    if (!char || llmRun.active) return;
    panelOpen = true;
    sectionByTab.characters = "names";
    await tick();
    await runNameAnalysis([$state.snapshot(char)], true);
  }

  export async function translateCharacter(alias: string) {
    const char = characters.find((char) => char.alias === alias);
    if (!char || llmRun.active) return;
    panelOpen = true;
    sectionByTab.characters = "names";
    await tick();
    if (!beginRun(`Переводим имя: ${characterTitle(char)}...`)) return;
    try {
      await loadNameRules();
      const chosen = [$state.snapshot(char)];
      const analysis = savedNameAnalysis(chosen);
      if (!analysis.complete) { errorMessage = "Сначала выполните анализ имени этого персонажа."; return; }
      if (missingNameRuleLanguages(analysis.origins).length > 0) { errorMessage = "Для имени ещё нет правил. Запустите анализ имени, чтобы подготовить их."; return; }
      await startNames(chosen);
    } catch (e) { errorMessage = `Не удалось перевести имя: ${String(e)}`; }
    finally { endRun(); }
  }

  function namePlan(origins: NameOrigin[] = analysisReady ? $state.snapshot(nameOrigins) : []) {
    return {
      origins,
      rules: [...new Set(origins.flatMap((origin) => origin.languages))].filter((language) => ruleDrafts[language]?.trim()).map((language) => ({ language, rules: ruleDrafts[language] })),
      glossary: [
        ...characters.flatMap((c) => [
          { source: c.first_name, translated: c.translated_first_name },
          { source: c.last_name, translated: c.translated_last_name },
          ...c.merged.map((part) => ({ source: part.original_name, translated: translatedTitle(c) })),
          ...c.forms.map((form) => ({ source: form.source, translated: formTranslation(form, translatedTitle(c)) }))
        ]),
        ...terms.flatMap((term) => [
          { source: term.source_term, translated: term.translated_term },
          ...term.forms.map((form) => ({ source: form.source, translated: formTranslation(form, term.translated_term) }))
        ])
      ].filter((form) => form.source.trim() && form.translated.trim())
    };
  }
  const FIELD_LABELS: Record<NameField, string> = { first: "Имя", last: "Фамилия", form: "Другая форма" };
  // Формы показываются рядом со своим именем, в какой бы момент ни пришёл ответ
  let reviewGroups = $derived(characters.flatMap((character) => {
    const rows = reviewNames.filter((row) => row.alias === character.alias);
    return rows.length > 0 ? [{ alias: character.alias, title: characterTitle(character), rows }] : [];
  }));
  let unchangedNames = $state(0);
  let namesRequested = $state(0);
  let namesFinished = $state(false);
  let chosenNames = $derived(reviewNames.filter((row) => row.translated.trim() && row.translated !== row.current));
  // Итог команды может опередить последний пакет из канала, поэтому подпись считается по строкам
  let namesNote = $derived.by(() => {
    if (!namesFinished) return "Новые написания появляются здесь по мере ответа модели.";
    const missing = namesRequested - reviewNames.length;
    const note = `Обработано записей: ${reviewNames.length}, без изменений: ${unchangedNames}`;
    return missing > 0 ? `${note}, не получено ответов: ${missing}.` : `${note}.`;
  });

  function gameCast() { return characters.flatMap((c) => [characterTitle(c), ...c.merged.map((m) => m.original_name)]); }

  /** Кладёт написания в отбор. Модель получает повторы один раз, а написание нужно каждому их владельцу. */
  function addNames(items: NameSuggestion[], targets: NameBatch["targets"]) {
    for (const item of items) {
      for (const target of targets.get(item.id) ?? []) {
        const options = nameOptions(item, target.current);
        if (options.length === 1 && options[0].translated === target.current) {
          unchangedNames += 1;
        }
        if (options.length > 0) reviewNames.push({ ...target, key: nameTargetKey(target), translated: target.current || "", origin: item.origin, options, notice: nameCheckNotice(item.issues || []), recommended: modelNameChoice(item, options) });
      }
    }
  }

  /**
   * Модель пишет имена выбранных персонажей, видя весь состав игры, а затем от них их формы.
   * Заполненные тоже участвуют, но заменятся, только если их отметить.
   */
  async function startNames(selected?: PanelCharacter[]) {
    const chosen = selected ?? $state.snapshot(nameTargets);
    const analysis = savedNameAnalysis(chosen);
    if (!analysis.complete || missingNameRuleLanguages(analysis.origins).length > 0) return;
    const plan = namePlan(analysis.origins);
    progressStep = "3–4. Переводим и проверяем имена и фамилии...";
    llmRun.note = progressStep;
    llmRun.progress = null;
    doneMessage = "";
    reviewNames = [];
    unchangedNames = 0;
    namesRequested = 0;
    namesFinished = false;
    const spelled = new Map<string, string>();
    let failedBatches = 0;
    const failures: { name: string; reason: string }[] = [];

    const spell = async ({ requests, targets }: NameBatch) => {
      if (requests.length === 0) return;
      namesRequested += [...targets.values()].flat().length;
      const received = new Set<string>();
      const receive = (items: NameSuggestion[]) => {
        for (const item of items) {
          if (received.has(item.id)) continue;
          received.add(item.id);
          addNames([item], targets);
        }
      };
      const result = await invoke<AutofillResult<NameSuggestion>>("translate_character_names", {
        target: targetFor(settings, "names"),
        names: requests,
        cast: gameCast(),
        plan,
        onProgress: requestProgressChannel(),
        onBatch: batchChannel<NameSuggestion>(receive)
      });
      receive(result.items);
      failedBatches += result.failed_batches;
      failures.push(...(result.errors ?? []));
      // Итог команды полный, а сообщения канала могут отстать от него
      for (const item of result.items) {
        for (const target of targets.get(item.id) ?? []) spelled.set(nameTargetKey(target), item.translated);
      }
    };

    try {
      await spell(baseNames(chosen));
      if (!stopRequested) {
        progressStep = "3–4. Переводим и проверяем другие формы имён...";
        llmRun.note = progressStep;
        await spell(formNames(chosen, spelled));
      }
      if (failedBatches > 0) errorMessage = failures.length > 0
        ? failures.slice(0, 3).map((failure) => `${failure.name}: ${failure.reason}`).join("; ") + (failedBatches > failures.length ? ` Пропущено пакетов: ${failedBatches - failures.length}.` : "")
        : failedBatchesMessage(failedBatches);
    } catch (e) {
      if (reviewNames.length === 0) namesRequested = 0;
      errorMessage = `Не удалось перевести имена: ${String(e)}`;
    } finally {
      namesFinished = true;
    }
  }

  // Ответ чат-бота приходит не сразу, а связан с тем запросом, который ушёл в чат
  let chatNameBatch: NameBatch | null = null;

  async function requestNamesByChat() {
    const batch = chatNames($state.snapshot(nameTargets));
    if (batch.requests.length === 0) {
      errorMessage = "У выбранных персонажей нет имён для перевода.";
      return;
    }
    if (await copyChatRequest("names", (async () => {
      await loadNameRules();
      const plan = namePlan();
      if (!analysisReady) plan.rules = nameRules.items.filter((rule) => rule.rules.trim()).map((rule) => ({ ...rule }));
      return invoke<string>("names_chat_request", { names: batch.requests, cast: gameCast(), plan });
    })())) {
      chatNameBatch = batch;
    }
  }

  async function readNamesAnswer() {
    if (!chatNameBatch) return;
    const { requests, targets } = chatNameBatch;
    errorMessage = null;
    doneMessage = "";
    try {
      const items = await invoke<NameSuggestion[]>("read_names_chat_answer", { names: requests, answer: chatAnswers.names });
      reviewNames = [];
      unchangedNames = 0;
      namesRequested = [...targets.values()].flat().length;
      namesFinished = true;
      addNames(items, targets);
    } catch (e) {
      errorMessage = `Не удалось разобрать ответ: ${String(e)}`;
    }
  }

  function applyNames() {
    onnames(chosenNames.map(({ alias, field, source, translated }) => ({ alias, field, source, translated: translated.trim() })));
    doneMessage = `Обновлено имён: ${chosenNames.length}. Описания сохранят старое написание, пока их не обновить.`;
    reviewNames = [];
    namesRequested = 0;
    clearChat("names");
  }

  // Перевод терминов
  // Без явного выбора в запуске непереведённые: готовый перевод нельзя заменить случайно
  let unselectedTerms = $state<string[] | null>(null);
  let termsDeselected = $derived(unselectedTerms ?? terms.filter((t) => t.translated_term.trim()).map((t) => t.id));
  let termTargets = $derived(terms.filter((t) => !termsDeselected.includes(t.id)));
  let reviewTranslations = $state<{ id: string; source: string; current: string; translated: string; category: string; notes: string; selected: boolean }[]>([]);
  let translationsRun = $state(false);
  let chosenTranslations = $derived(reviewTranslations.filter((row) => row.selected && row.translated.trim()));

  function addTerms(items: PanelTerm[]) {
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
  }

  async function startTerms() {
    if (!beginRun("Модель переводит термины...")) return;
    reviewTranslations = [];
    translationsRun = true;
    try {
      const result = await invoke<AutofillResult<PanelTerm>>("autofill_glossary_terms", {
        target: targetFor(settings, "terms"),
        terms: $state.snapshot(termTargets),
        onBatch: batchChannel<PanelTerm>(addTerms)
      });
      if (result.failed_batches > 0) errorMessage = failedBatchesMessage(result.failed_batches);
    } catch (e) {
      errorMessage = `Не удалось обработать термины: ${String(e)}`;
    } finally {
      endRun();
    }
  }

  let chatTerms: PanelTerm[] = [];

  async function requestTermsByChat() {
    const chosen = $state.snapshot(termTargets);
    if (await copyChatRequest("terms", invoke<string>("terms_chat_request", { terms: chosen }))) {
      chatTerms = chosen;
    }
  }

  async function readTermsAnswer() {
    errorMessage = null;
    doneMessage = "";
    try {
      const items = await invoke<PanelTerm[]>("read_terms_chat_answer", { terms: chatTerms, answer: chatAnswers.terms });
      reviewTranslations = [];
      translationsRun = true;
      addTerms(items);
    } catch (e) {
      errorMessage = `Не удалось разобрать ответ: ${String(e)}`;
    }
  }

  function applyTranslations() {
    onterms(chosenTranslations.map(({ id, translated, category, notes }) => ({ id, translated: translated.trim(), category, notes })));
    doneMessage = `Обновлено терминов: ${chosenTranslations.length}`;
    reviewTranslations = [];
    translationsRun = false;
    clearChat("terms");
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

  // Подписи без реплик модели показать нечего. Без явного выбора в запуске подписи без перевода
  let readableSpeakers = $derived(speakers.filter((s) => s.lines > 0));
  let unselectedSpeakers = $state<string[] | null>(null);
  let speakersDeselected = $derived(unselectedSpeakers ?? readableSpeakers.filter((s) => s.translated.trim()).map((s) => s.name));
  let speakerTargets = $derived(readableSpeakers.filter((s) => !speakersDeselected.includes(s.name)));
  let reviewSpeakers = $state<(SpeakerSuggestion & { current: string; selected: boolean })[]>([]);
  let speakersRun = $state(false);
  let chosenSpeakers = $derived(reviewSpeakers.filter((s) => s.selected && s.translated.trim()));

  async function startSpeakers() {
    const names = speakerTargets.map((s) => s.name);
    if (!beginRun("Модель переводит подписи говорящих...")) return;
    reviewSpeakers = [];
    speakersRun = true;
    try {
      const result = await invoke<AutofillResult<SpeakerSuggestion>>("suggest_glossary_speakers", {
        gameDir,
        target: targetFor(settings, "speakers"),
        names,
        hints: $state.snapshot(hints),
        onBatch: batchChannel<SpeakerSuggestion>((items) => {
          reviewSpeakers.push(...items.map((item) => {
            const current = speakers.find((s) => s.name === item.name)?.translated ?? "";
            // Готовый перевод заменяется, только если его отметил пользователь
            return { ...item, current, selected: !current.trim() };
          }));
        })
      });
      if (result.failed_batches > 0) errorMessage = failedBatchesMessage(result.failed_batches);
    } catch (e) {
      errorMessage = `Не удалось перевести подписи: ${String(e)}`;
    } finally {
      endRun();
    }
  }

  function applySpeakers() {
    onspeakers(chosenSpeakers.map(({ name, translated, gender }) => ({ name, translated: translated.trim(), gender })));
    doneMessage = `Обновлено подписей: ${chosenSpeakers.length}`;
    reviewSpeakers = [];
    speakersRun = false;
  }
</script>

{#snippet runButton(label: string, disabled: boolean, onclick: () => void)}
  <button class="btn-run" {onclick} disabled={disabled || llmRun.active}>
    <Sparkles size={13} />
    <span>{label}</span>
  </button>
{/snippet}

{#snippet chatButton(disabled: boolean, onclick: () => void)}
  <button class="btn-run btn-chat" {onclick} disabled={disabled || llmRun.active} title="Перевод в чат-боте вручную, без API">
    <ClipboardCopy size={13} />
    <span>Скопировать запрос для чат-бота</span>
  </button>
{/snippet}

{#snippet chatAnswer(section: ChatSection, onread: () => void)}
  {#if chatRequests[section]}
    <div class="ai-review">
      <span class="ai-note">Запрос скопирован. Вставьте его в чат-бот, а ответ сюда.</span>
      <textarea class="input ai-answer" rows="4" bind:value={chatAnswers[section]} placeholder="Ответ чат-бота с JSON" aria-label="Ответ чат-бота"></textarea>
      <button class="btn-run btn-read" onclick={onread} disabled={!chatAnswers[section].trim() || llmRun.active}>
        <ClipboardPaste size={13} />
        <span>Разобрать ответ</span>
      </button>
    </div>
  {/if}
{/snippet}

{#snippet categorySelect(row: { category: string }, label: string)}
  <select class="input" bind:value={row.category} aria-label={`Категория термина ${label}`}>
    <option value="general">Общее</option>
    <option value="location">Локация</option>
    <option value="organization">Организация</option>
    <option value="title">Титул</option>
    <option value="item">Предмет</option>
    <option value="event">Событие</option>
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
        {#if progress && !llmRun.progress}
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

  {#if llmRun.active}<div class="ai-progress-detail"><LlmProgress /></div>{/if}
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
        <p class="ai-note">Массовый анализ заполняет только отсутствующие происхождения и правила. Затем можно переключить модель и запустить перевод.</p>
        <EntryPicker
          items={characters.map((c) => ({ id: c.alias, name: characterTitle(c) }))}
          unselected={unselectedAliases}
          disabled={llmRun.active}
          onchange={(next) => unselectedAliases = next}
          onempty={() => unselectedAliases = characters.filter((c) => translatedTitle(c)).map((c) => c.alias)}
        />
        <div class="ai-actions">
          {@render runButton(`Анализ и правила (${nameTargets.length})`, nameTargets.length === 0, startNameAnalysis)}
          {@render runButton(`Перевести имена (${nameTargets.length})`, !analysisReady || missingRules.length > 0, startNameTranslation)}
          {@render chatButton(nameTargets.length === 0, requestNamesByChat)}
        </div>
        <p class="ai-note">В ручном чате этапы объединены в один запрос с сохранёнными правилами. Отдельная проверка через API выполняется при запуске «Перевести имена».</p>
        {@render chatAnswer("names", readNamesAnswer)}

        {#if namesRequested > 0}
          <div class="ai-review">
            <div class="ai-review-head">
              <span class="ai-note">{namesNote}</span>
              {#if reviewNames.length > 0}
                <div class="ai-toggles">
                  <button class="btn-link" onclick={() => { for (const row of reviewNames) row.translated = row.recommended; }}>Выбрать предложения модели</button>
                  <button class="btn-link" onclick={() => { for (const row of reviewNames) row.translated = row.current || ""; }}>Снять выбор</button>
                </div>
              {/if}
            </div>
            {#each reviewGroups as group (group.alias)}
              <span class="ai-group">{group.title}</span>
              {#each group.rows as row (row.key)}
                <div class="ai-name-choice">
                  <div class="ai-source">
                    <span class="ai-term">{row.source} <span class="ai-count">{FIELD_LABELS[row.field]}</span></span>
                    {#if row.origin}<span class="ai-example">{row.origin}</span>{/if}
                    <span class="ai-example">Сейчас: {row.current || "нет перевода"}</span>
                    {#if row.notice}<span class="ai-example">{row.notice}</span>{/if}
                  </div>
                  <fieldset class="ai-name-options">
                    <legend class="sr-only">Написание {row.source}</legend>
                    {#each row.options as option (option.translated)}
                      <label class="ai-name-option">
                        <input type="radio" name={`name-${row.key}`} value={option.translated} checked={row.translated === option.translated} onchange={() => row.translated = option.translated} />
                        <span><strong>{option.translated}</strong><span class="ai-example">{option.description}</span></span>
                      </label>
                    {/each}
                    {#if !row.current}
                      <label class="ai-name-option"><input type="radio" name={`name-${row.key}`} value="" checked={row.translated === ""} onchange={() => row.translated = ""} /><span>Не применять</span></label>
                    {/if}
                  </fieldset>
                </div>
              {/each}
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
        <div class="ai-actions">
          {@render runButton(`Перевести термины (${termTargets.length})`, termTargets.length === 0, startTerms)}
          {@render chatButton(termTargets.length === 0, requestTermsByChat)}
        </div>
        {@render chatAnswer("terms", readTermsAnswer)}

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
      {:else if openSection === "speakers"}
        <p class="ai-note">Модель читает несколько реплик каждого говорящего и предлагает перевод подписи и род. Что записать в глоссарий, выберете вы.</p>
        <EntryPicker
          items={readableSpeakers.map((s) => ({ id: s.name, name: s.name }))}
          unselected={speakersDeselected}
          disabled={llmRun.active}
          onchange={(next) => unselectedSpeakers = next}
          onempty={() => unselectedSpeakers = null}
        />
        {@render runButton(`Перевести подписи (${speakerTargets.length})`, speakerTargets.length === 0, startSpeakers)}

        {#if speakersRun && !isProcessing && reviewSpeakers.length === 0 && !errorMessage}
          <p class="ai-note">Модель не смогла ничего предложить.</p>
        {/if}
        {#if reviewSpeakers.length > 0}
          <div class="ai-review">
            {#each reviewSpeakers as row (row.name)}
              <div class="ai-row ai-row-select" class:off={!row.selected}>
                <input type="checkbox" bind:checked={row.selected} aria-label={`Записать перевод ${row.name}`} />
                <div class="ai-source">
                  <span class="ai-term">{row.name}</span>
                  <span class="ai-example">Сейчас: {row.current || "нет перевода"}</span>
                </div>
                <input type="text" class="input" bind:value={row.translated} placeholder="Перевод" aria-label={`Перевод подписи ${row.name}`} />
                <select class="input" bind:value={row.gender} aria-label={`Род говорящего ${row.name}`}>
                  <option value="male">Мужской</option>
                  <option value="female">Женский</option>
                  <option value="unknown">Не указан</option>
                </select>
              </div>
            {/each}
            <button class="btn-apply" onclick={applySpeakers} disabled={chosenSpeakers.length === 0 || isProcessing}>
              <Check size={14} />
              <span>Записать выбранные ({chosenSpeakers.length})</span>
            </button>
          </div>
        {/if}
      {/if}
    </div>
  {/if}
</div>

<style>
  .ai-progress-detail { padding: 7px 14px; border-top: 1px solid rgba(99, 102, 241, .25); }
  .ai-name-choice { display: grid; grid-template-columns: minmax(140px, 1fr) minmax(220px, 2fr); gap: 16px; padding: 12px 0; border-bottom: 1px solid #263247; }
  .ai-name-options { border: 0; margin: 0; padding: 0; display: grid; gap: 8px; min-width: 0; }
  .ai-name-option { display: flex; align-items: flex-start; gap: 9px; cursor: pointer; font-size: 12px; color: #cbd5e1; }
  .ai-name-option input { margin-top: 3px; accent-color: #818cf8; }
  .ai-name-option strong { display: block; font-weight: 500; color: #e2e8f0; }
  .sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); }
  @media (max-width: 700px) { .ai-name-choice { grid-template-columns: 1fr; } }
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

  .ai-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }

  /* Запрос в чат-бот не запускает модель, поэтому кнопка тише запуска */
  .btn-chat {
    border-color: rgba(71, 85, 105, 0.6);
    background: transparent;
    color: #cbd5e1;
  }

  .ai-answer {
    font-family: inherit;
    resize: vertical;
  }

  /* Завершает блок ответа там же, где отбор завершает кнопка записи */
  .btn-read {
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

  .ai-group {
    margin-top: 6px;
    font-size: 11px;
    font-weight: 600;
    color: #94a3b8;
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
