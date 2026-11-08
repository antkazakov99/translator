<script lang="ts">
  import { onMount } from "svelte";
  import { Channel } from "@tauri-apps/api/core";
  import { LoaderCircle, Plus, Save, Sparkles, Square } from "lucide-svelte";
  import { commands, type LanguageRulesDto_Serialize as LanguageRules, type RunProgress_Deserialize as RunProgress } from "./bindings";
  import { languageName, matches, runsBusy, type GlossaryRuns } from "./glossary";

  let { projectId, runs, onError }: { projectId: string; runs: GlossaryRuns; onError: (message: string) => void } = $props();

  let languages = $state<LanguageRules[]>([]);
  let loading = $state(true);
  let query = $state("");
  let newCode = $state("");
  let selected = $state<string | null>(null);
  /** Поля в правке по коду языка: правка не теряется при переходе к другому языку. */
  let edits = $state<Record<string, { rules: string; own: string }>>({});
  let saving = $state(false);
  let drafting = $state(false);
  let stopping = $state(false);
  let notice = $state("");

  onMount(load);

  async function load() {
    try {
      languages = await commands.nameRules(projectId);
    } catch (e) {
      onError(String(e));
    } finally {
      loading = false;
    }
  }

  const shown = $derived(languages.filter((item) => matches(query, item.language, languageName(item.language))));
  const current = $derived(languages.find((item) => item.language === (selected ?? shown[0]?.language)) ?? null);
  const edit = $derived(current ? (edits[current.language] ?? { rules: current.rules ?? "", own: current.own ?? "" }) : null);
  const dirty = $derived(Boolean(current && edit && (edit.rules !== (current.rules ?? "") || edit.own !== (current.own ?? ""))));

  function status(item: LanguageRules) {
    if (item.own) return "свои правила проекта";
    if (item.rules) return "общие правила";
    return "нет правил";
  }

  /** Поля текущего языка для правки: копия сохранённого при первом касании. */
  function touch(): { rules: string; own: string } | null {
    if (!current) return null;
    edits[current.language] ??= { rules: current.rules ?? "", own: current.own ?? "" };
    return edits[current.language];
  }

  function add() {
    const code = newCode.trim().replace(/_/g, "-");
    if (!code) return;
    const known = languages.find((item) => item.language.toLowerCase() === code.toLowerCase());
    if (!known) languages = [...languages, { language: code, names: 0, rules: null, own: null }];
    selected = known?.language ?? code;
    newCode = "";
    query = "";
    notice = "";
  }

  async function save() {
    if (!current || !edit) return;
    const language = current.language;
    const { rules, own } = $state.snapshot(edit);
    saving = true;
    try {
      if (rules !== (current.rules ?? "")) await commands.saveNameRule(projectId, language, rules);
      if (own !== (current.own ?? "")) await commands.saveOwnNameRule(projectId, language, own);
      delete edits[language];
      await load();
      selected = languages.some((item) => item.language === language) ? language : null;
      notice = "Правила сохранены";
    } catch (e) {
      onError(String(e));
    } finally {
      saving = false;
    }
  }

  async function draft() {
    const fields = touch();
    if (!current || !fields) return;
    const language = current.language;
    drafting = true;
    stopping = false;
    notice = "";
    try {
      const channel = new Channel<RunProgress>();
      const result = await commands.draftNameRule(projectId, language, fields.rules.trim() || null, null, channel);
      if (result.error) onError(result.error);
      if (result.text) {
        edits[language] = { ...(edits[language] ?? fields), rules: result.text };
        notice = "Черновик готов: проверьте его и сохраните";
      } else if (!result.error) {
        notice = stopping ? "Остановлено" : "Модель ничего не написала";
      }
    } catch (e) {
      onError(String(e));
    } finally {
      drafting = false;
    }
  }

  async function stop() {
    stopping = true;
    try {
      await commands.cancelLlmRun();
    } catch (e) {
      onError(String(e));
    }
  }
</script>

<div class="toolbar">
  <div class="toolbar-title">
    <strong>Правила имён</strong>
    <span>Как передавать по-русски имена каждого языка. Перевод имён берёт правила языков, которые нашёл разбор происхождения</span>
  </div>
</div>

{#if drafting}
  <div class="banner progress page-error" role="status">
    <LoaderCircle size={14} class="spin" />
    <span>{stopping ? "Черновик правил: останавливается" : `Черновик правил: ${current ? languageName(current.language) : ""}`}</span>
    <button class="btn small" disabled={stopping} onclick={stop}>
      <Square size={13} />
      <span>Остановить</span>
    </button>
  </div>
{/if}

<div class="master">
  <div class="list-side">
    <input class="input" placeholder="Поиск по языку" bind:value={query} aria-label="Поиск языков" />
    <div class="items">
      {#if loading}
        <p class="faint">Загрузка правил</p>
      {:else}
        {#each shown as item (item.language)}
          <button class="item" class:active={item.language === current?.language} onclick={() => (selected = item.language)}>
            <strong>{languageName(item.language)} <span class="code">{item.language}</span></strong>
            <span class:missing={!item.rules && !item.own}>{item.names ? `имён: ${item.names} · ` : ""}{status(item)}</span>
          </button>
        {:else}
          <p class="faint">{languages.length ? "Ничего не нашлось" : "Языков пока нет: их найдёт перевод имён, или добавьте язык по коду ниже"}</p>
        {/each}
      {/if}
    </div>
    <form class="add" onsubmit={(event) => (event.preventDefault(), add())}>
      <input class="input" placeholder="Код языка: sr, de, ja" bind:value={newCode} aria-label="Код нового языка" />
      <button class="btn small" type="submit" disabled={!newCode.trim()}>
        <Plus size={12} />
        <span>Добавить</span>
      </button>
    </form>
  </div>

  {#if current && edit}
    <div class="card editor" class:saving>
      <div class="card-head">
        <strong>{languageName(current.language)} · {current.language}</strong>
        <button
          class="btn small"
          disabled={drafting || runsBusy(runs)}
          title={runsBusy(runs) ? "Модель уже работает или ждёт отбор" : "Модель составит правила по общим правилам этого языка и дополнит то, что уже написано"}
          onclick={draft}
        >
          {#if drafting}<LoaderCircle size={13} class="spin" />{:else}<Sparkles size={13} />{/if}
          <span>Черновик моделью</span>
        </button>
      </div>

      <label class="field">
        Общие правила, для всех проектов
        <textarea
          class="input"
          rows="10"
          value={edit.rules}
          oninput={(event) => {
            const fields = touch();
            if (fields) fields.rules = event.currentTarget.value;
          }}
          placeholder="Например: lj и nj читаются мягко, j это й, c это ц"
        ></textarea>
      </label>

      <label class="field">
        Свои правила этого проекта, важнее общих
        <textarea
          class="input"
          rows="4"
          value={edit.own}
          oninput={(event) => {
            const fields = touch();
            if (fields) fields.own = event.currentTarget.value;
          }}
          placeholder="Если в игре своя традиция написания имён этого языка"
        ></textarea>
      </label>

      <div class="card-actions">
        {#if notice}<span class="faint">{notice}</span>{/if}
        <button class="btn small primary" disabled={!dirty || saving || drafting} onclick={save}>
          {#if saving}<LoaderCircle size={13} class="spin" />{:else}<Save size={13} />{/if}
          <span>Сохранить</span>
        </button>
      </div>
      <p class="faint">Пустое поле удаляет правила при сохранении.</p>
    </div>
  {:else}
    <div class="empty">
      <strong>Язык не выбран</strong>
      <span>Выберите язык в списке или добавьте его по коду</span>
    </div>
  {/if}
</div>

<style>
  .code {
    color: var(--text-faint);
    font-family: var(--font-mono);
    font-size: 11px;
    font-weight: 400;
  }

  .item > .missing {
    color: var(--warning);
  }
</style>
