<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { AlertCircle, Check, Loader2, Plus, Save, Search, Sparkles, Square, Trash2 } from "lucide-svelte";
  import LlmChoice from "./LlmChoice.svelte";
  import { beginLlmRun, endLlmRun, llmRun, setLlmProgress } from "./llm-run.svelte";
  import LlmProgress from "./LlmProgress.svelte";
  import { targetFor, type LlmAction, type LlmActionSettings, type LlmSettings } from "./llm-settings";
  import { nameRules, dirtyNameRuleLanguages, loadNameRules, saveNameRules, setNameRule, removeNameRule, renameNameRule, normalizeLanguageName, type LanguageRule } from "./name-rules.svelte";

  let { settings, onselect }: {
    settings: LlmSettings;
    onselect: (action: LlmAction, choice: LlmActionSettings) => void;
  } = $props();

  let search = $state("");
  let language = $state("");
  let selected = $state("");
  let renameDraft = $state("");
  let deletedRule = $state<LanguageRule | null>(null);
  let error = $state("");
  let done = $state("");
  let generating = $state(false);
  let stopping = $state(false);
  let dirty = $derived(dirtyNameRuleLanguages());
  let filtered = $derived(nameRules.items.filter((rule) => rule.language.includes(search.trim().toLowerCase())).toSorted((a, b) => a.language.localeCompare(b.language, "ru")));
  let current = $derived(nameRules.items.find((rule) => rule.language === selected));
  $effect(() => { renameDraft = selected; });

  onMount(() => {
    void loadNameRules().then(() => { if (!selected) selected = nameRules.items[0]?.language ?? ""; })
      .catch((e) => error = `Не удалось загрузить правила: ${String(e)}`);
  });
  onDestroy(() => { if (generating) { stopping = true; void invoke("cancel_llm_run"); } });

  function addLanguage() {
    const key = normalizeLanguageName(language);
    if (!key) return;
    if (["описание", "неизвестно"].includes(key)) {
      error = "Укажите язык происхождения или название вымышленного языка.";
      return;
    }
    if (!nameRules.items.some((rule) => rule.language === key)) setNameRule(key, "");
    selected = key;
    language = "";
    search = "";
    error = "";
    done = "";
  }

  async function generate() {
    if (!current || !beginLlmRun(`Правила передачи имён: ${current.language}`)) return;
    const key = current.language;
    setLlmProgress({ phase: "rules", batch: 1, total_batches: 1, completed: 0, total: 1, attempt: 1 });
    generating = true;
    stopping = false;
    error = "";
    done = "";
    try {
      await loadNameRules();
      const existingRules = nameRules.items.find((rule) => rule.language === key)?.rules ?? "";
      const rule = await invoke<LanguageRule>("generate_name_rules", { language: key, existingRules, target: targetFor(settings, "names") });
      if (!stopping) {
        setNameRule(key, rule.rules);
        done = "Черновик готов. Проверьте и сохраните правила.";
      }
    } catch (e) { error = `Не удалось создать правила: ${String(e)}`; }
    finally { generating = false; endLlmRun(); }
  }

  async function save() {
    error = "";
    done = "";
    try {
      await saveNameRules();
      deletedRule = null;
      done = "Правила сохранены и доступны во всех проектах.";
    } catch (e) { error = `Не удалось сохранить правила: ${String(e)}`; }
  }

  function rename() {
    if (!current || llmRun.active || nameRules.saving || nameRules.loading) return;
    error = "";
    done = "";
    try {
      renameNameRule(selected, renameDraft);
      selected = normalizeLanguageName(renameDraft);
    } catch (e) { error = String(e); }
  }

  function remove() {
    if (!current || llmRun.active || nameRules.saving || nameRules.loading) return;
    deletedRule = { ...current };
    removeNameRule(selected);
    selected = filtered[0]?.language ?? "";
    error = "";
    done = "";
  }

  function undoRemove() {
    if (!deletedRule) return;
    if (nameRules.items.some((rule) => rule.language === deletedRule?.language)) {
      error = "Правила для этого языка уже созданы. Восстановление заменило бы новые правки.";
      return;
    }
    setNameRule(deletedRule.language, deletedRule.rules);
    selected = deletedRule.language;
    deletedRule = null;
  }
</script>

<section class="rules-library" aria-label="Правила передачи имён">
  <div class="rules-heading">
    <div><h4>Правила передачи имён на русский</h4><p>Общая библиотека для всех проектов. Перевод имён подбирает правила по найденным языкам.</p></div>
    <button onclick={save} disabled={nameRules.loading || nameRules.saving || llmRun.active || dirty.length === 0}><Save size={14} /> {nameRules.saving ? "Сохранение..." : `Сохранить правила${dirty.length ? ` (${dirty.length})` : ""}`}</button>
  </div>
  {#if error}<p class="message error"><AlertCircle size={14} />{error}</p>{/if}
  {#if done}<p class="message success"><Check size={14} />{done}</p>{/if}
  {#if deletedRule}<div class="message">Удалены правила: {deletedRule.language}. <button onclick={undoRemove} disabled={nameRules.saving || llmRun.active}>Отменить удаление</button></div>{/if}
  {#if llmRun.active}<div class="run-status"><p class="message"><Loader2 class="spin" size={14} />{llmRun.note}</p><LlmProgress /></div>{/if}
  <div class="rules-layout">
    <aside>
      <label class="search"><Search size={14} /><input bind:value={search} placeholder="Поиск по языку" aria-label="Поиск по языку" /></label>
      <form onsubmit={(event) => { event.preventDefault(); addLanguage(); }}>
        <input bind:value={language} placeholder="Язык, например сербский" aria-label="Новый язык правил" disabled={llmRun.active || nameRules.loading || nameRules.saving} />
        <button disabled={!language.trim() || llmRun.active || nameRules.loading || nameRules.saving} title="Добавить язык"><Plus size={16} /></button>
      </form>
      <nav aria-label="Языки правил">
        {#if nameRules.loading}<p class="message"><Loader2 class="spin" size={14} />Загружаем правила...</p>{/if}
        {#each filtered as rule (rule.language)}
          <button class:chosen={selected === rule.language} onclick={() => { selected = rule.language; done = ""; }} disabled={generating}>
            <span>{rule.language}</span><small>{dirty.includes(rule.language) ? "Не сохранено" : rule.rules.trim() ? "Сохранено" : "Нет правил"}</small>
          </button>
        {/each}
        {#if !nameRules.loading && filtered.length === 0}<p class="hint">{nameRules.items.length ? "Язык не найден." : "Сохранённых правил пока нет. Добавьте язык или сгенерируйте правила при переводе имён."}</p>{/if}
      </nav>
    </aside>
    <div class="rules-editor">
      {#if current}
        <div class="editor-heading"><h4>{current.language}</h4><LlmChoice action="names" {settings} {onselect} /></div>
        <label for="rule-language">Название языка</label>
        <form class="rename-form" onsubmit={(event) => { event.preventDefault(); rename(); }}>
          <input id="rule-language" bind:value={renameDraft} disabled={llmRun.active || nameRules.saving || nameRules.loading} />
          <button disabled={llmRun.active || nameRules.saving || nameRules.loading || !renameDraft.trim() || renameDraft.trim().toLowerCase() === selected}>Переименовать</button>
        </form>
        <label for="language-rules">Правила и примеры написания</label>
        <textarea id="language-rules" value={current.rules} oninput={(event) => setNameRule(selected, event.currentTarget.value)} disabled={llmRun.active || nameRules.saving} placeholder="Введите правила или запросите черновик у модели."></textarea>
        <div class="editor-actions">
          <button onclick={generate} disabled={llmRun.active || nameRules.loading || nameRules.saving}><Sparkles size={14} />{current.rules.trim() ? "Сгенерировать заново" : "Сгенерировать правила"}</button>
          <button class="delete-rule" onclick={remove} disabled={llmRun.active || nameRules.loading || nameRules.saving}><Trash2 size={14} />Удалить правила</button>
          {#if dirty.includes(selected)}<button onclick={() => setNameRule(selected, nameRules.saved[selected] ?? "")} disabled={llmRun.active || nameRules.saving}>Отменить правки</button>{/if}
          {#if generating}<button onclick={async () => { stopping = true; await invoke("cancel_llm_run"); }} disabled={stopping}><Square size={12} />Остановить</button><span class="message"><Loader2 class="spin" size={14} />{stopping ? "Дожидаемся текущего ответа..." : "Готовим черновик..."}</span>{/if}
        </div>
        <p class="hint">Ответ модели — черновик: проверьте его перед сохранением. Правки сразу доступны в переводе имён. Сохранение запишет текст, новое название языка и удаления для следующих запусков приложения.</p>
      {:else}<p class="empty">Выберите язык слева или добавьте новый.</p>{/if}
    </div>
  </div>
</section>

<style>
  .rules-library { flex: 1; min-height: 0; display: flex; flex-direction: column; gap: 14px; padding: 20px; color: #cbd5e1; overflow: auto; }
  .rules-heading, .editor-heading { display: flex; align-items: center; justify-content: space-between; gap: 16px; flex-wrap: wrap; }
  h4 { margin: 0; font-size: 15px; color: #e2e8f0; }
  p { margin: 8px 0 0; font-size: 12px; line-height: 1.6; }
  .rules-heading p, .hint { color: #94a3b8; }
  button { display: inline-flex; align-items: center; justify-content: center; gap: 7px; padding: 8px 12px; border: 1px solid #334155; border-radius: 6px; background: #182131; color: #e2e8f0; font: inherit; font-size: 12px; cursor: pointer; }
  button:hover:not(:disabled) { border-color: #818cf8; background: #25264b; }
  button:disabled { opacity: .5; cursor: default; }
  .rules-layout { flex: 1; min-height: 340px; display: grid; grid-template-columns: 265px minmax(0, 1fr); border: 1px solid #263247; border-radius: 8px; overflow: hidden; }
  aside { padding: 14px; border-right: 1px solid #263247; background: #101622; }
  input, textarea { box-sizing: border-box; min-width: 0; padding: 9px 10px; border: 1px solid #334155; border-radius: 5px; background: #090e17; color: #e2e8f0; font: inherit; font-size: 13px; }
  input:focus, textarea:focus { outline: 1px solid #818cf8; }
  .search { display: flex; align-items: center; gap: 8px; }
  .search input { width: 100%; }
  form { display: flex; gap: 6px; margin: 12px 0; }
  form input { flex: 1; }
  .rename-form { margin: 0; }
  .delete-rule { color: #fda4af; border-color: #7f1d3c; }
  nav { display: grid; align-content: start; gap: 6px; }
  nav button { justify-content: space-between; text-align: left; }
  nav button.chosen { border-color: #818cf8; background: #25264b; }
  small { color: #94a3b8; font-size: 10px; }
  .rules-editor { display: flex; flex-direction: column; gap: 12px; padding: 18px; min-width: 0; }
  .rules-editor label { color: #94a3b8; font-size: 12px; }
  textarea { width: 100%; flex: 1; min-height: 240px; resize: vertical; line-height: 1.65; }
  .editor-actions { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .message { display: flex; gap: 7px; align-items: center; font-size: 12px; }
  .error { color: #fda4af; } .success { color: #34d399; }
  .empty { margin: auto; color: #94a3b8; }
  .rules-library :global(.spin) { animation: spin 1s linear infinite; } @keyframes spin { to { transform: rotate(360deg); } }
  @media (max-width: 800px) { .rules-layout { grid-template-columns: 1fr; } aside { border-right: 0; border-bottom: 1px solid #263247; } }
</style>
