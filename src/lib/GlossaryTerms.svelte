<script lang="ts">
  import DescribeParts from "./DescribeParts.svelte";
  import EntryLines from "./EntryLines.svelte";
  import { Check, Languages, LoaderCircle, Plus, Trash2, Undo2, UserRound } from "lucide-svelte";
  import Combobox from "./Combobox.svelte";
  import GlossaryForms from "./GlossaryForms.svelte";
  import SuggestStatus from "./SuggestStatus.svelte";
  import { commands, type EntryLineDto_Serialize as EntryLine, type TermCategory } from "./bindings";
  import {
    categories,
    categoryLabel,
    characterName,
    matches,
    runsBusy,
    type Glossary,
    type GlossaryRuns,
    type NameForm,
    type Term,
  } from "./glossary";

  let {
    projectId,
    data,
    runs,
    selected = $bindable(null),
    reload,
    patch,
    onOpenLine,
    onError,
  }: {
    projectId: string;
    data: Glossary;
    runs: GlossaryRuns;
    selected: number | null;
    reload: () => Promise<void>;
    patch: (term: Term) => void;
    /** Реплика термина в таблице перевода или в исходнике. */
    onOpenLine: (line: EntryLine, where: "table" | "source") => void;
    onError: (message: string) => void;
  } = $props();

  const run = $derived(runs.terms);
  const untranslated = $derived(data.terms.filter((term) => !term.translation?.trim()));
  let writing = $state(false);

  function runTitle() {
    if (untranslated.length === 0) return "У всех терминов есть перевод";
    if (runsBusy(runs)) return "Модель уже работает или ждёт отбор";
    return "Модель предложит перевод терминам без перевода, а термину общей категории и категорию";
  }

  function startRun() {
    const ids = untranslated.map((term) => term.id);
    run.start((channel) => commands.suggestTerms(projectId, ids, null, channel), onError);
  }

  /** Отмеченное пишет `accept_terms`, та же запись, что у автоперевода: перевод ложится только в пустое поле. */
  async function writeChosen() {
    const chosen = run.chosen
      .filter((item) => item.translation.trim())
      .map((item) => ({ id: item.id, translation: item.translation.trim(), category: item.category }));
    writing = true;
    try {
      const written = await commands.acceptTerms(projectId, chosen);
      run.finish(`Записано терминов: ${written}`);
      await reload();
    } catch (e) {
      onError(String(e));
    } finally {
      writing = false;
    }
  }

  type Filter = "all" | "trash";
  let filter = $state<Filter>("all");

  let query = $state("");
  let newName = $state("");
  let creating = $state(false);
  let saving = $state(false);
  let fieldError = $state("");
  /** Название в корзине, про которое открыт вопрос об окончательном удалении. */
  let confirmForget = $state<string | null>(null);
  /** Открыт вопрос о сбросе заметок модели: остальные действия шапки на это время скрыты. */
  let resetting = $state(false);
  let giving = $state(false);
  let giveText = $state("");
  let draft = $state<Term | null>(null);

  const shown = $derived(
    data.terms
      .filter((term) => matches(query, term.name, term.translation, ...term.forms.map((form) => form.text))),
  );

  /** Сохранённое название и формы: по ним ищутся реплики термина. */
  const linesRevision = $derived.by(() => {
    const saved = data.terms.find((term) => term.id === draft?.id);
    return JSON.stringify([saved?.name, saved?.forms.map((form) => form.text)]);
  });

  $effect.pre(() => {
    const id = selected ?? shown[0]?.id ?? null;
    if (draft?.id === id) return;
    const found = data.terms.find((term) => term.id === id);
    draft = found ? structuredClone($state.snapshot(found)) : null;
    fieldError = "";
    giving = false;
  });

  /** Модель записала описание: оно ложится в открытую карточку. */
  async function described() {
    await reload();
    const found = data.terms.find((term) => term.id === draft?.id);
    if (draft && found) draft.description = found.description;
  }

  async function save(next?: Partial<Term>) {
    if (!draft) return;
    Object.assign(draft, next);
    const snapshot = $state.snapshot(draft);
    saving = true;
    try {
      await commands.saveTerm(projectId, snapshot);
      patch(snapshot);
      fieldError = "";
    } catch (e) {
      fieldError = String(e);
    } finally {
      saving = false;
    }
  }

  async function act(work: () => Promise<unknown>, after?: () => void) {
    try {
      await work();
      after?.();
      await reload();
    } catch (e) {
      onError(String(e));
    }
  }

  async function create() {
    const name = newName.trim();
    if (!name) return;
    creating = true;
    try {
      const id = await commands.takeTerm(projectId, name, "general");
      newName = "";
      filter = "all";
      await reload();
      selected = id;
    } catch (e) {
      onError(String(e));
    } finally {
      creating = false;
    }
  }

  const characterOptions = (all: boolean) =>
    data.characters
      .filter((character) => all || matches(giveText, characterName(character)))
      .map((character) => ({ id: String(character.id), label: characterName(character) }));

  function forget() {
    selected = null;
    draft = null;
  }

  /** Единственное удаление записи: термин уходит в корзину вместе с полями. Окончательно его удаляют уже там. */
  function toTrash() {
    const term = draft;
    if (!term) return;
    act(() => commands.trashTerm(projectId, term.id), forget);
  }
</script>

<div class="toolbar">
  <div class="toolbar-title">
    <strong>Термины</strong>
    <span>Места, организации, предметы и звания. Новые находит модель, когда описывает сцену</span>
  </div>
  <div class="segmented" role="tablist" aria-label="Какие термины">
    <button role="tab" aria-selected={filter === "all"} class:on={filter === "all"} onclick={() => (filter = "all")}>Все · {data.terms.length}</button>
    <button role="tab" aria-selected={filter === "trash"} class:on={filter === "trash"} onclick={() => (filter = "trash")}>
      Корзина · {data.discardedTerms.length}
    </button>
  </div>
  <button class="btn small primary" disabled={untranslated.length === 0 || runsBusy(runs)} title={runTitle()} onclick={startRun}>
    {#if run.active}<LoaderCircle size={13} class="spin" />{:else}<Languages size={13} />{/if}
    <span>Перевести · {untranslated.length}</span>
  </button>
</div>

<SuggestStatus {run} {onError} />
<SuggestStatus run={runs.entries} {onError} />

{#if run.review}
  <div class="list">
    <p class="pick-head">Предложения модели. Отмеченные запишутся в глоссарий, перевод и категорию можно поправить</p>
    {#each run.review as item (item.id)}
      {@const term = data.terms.find((known) => known.id === item.id)}
      <div class="pick-row terms" class:off={!item.selected}>
        <input type="checkbox" bind:checked={item.selected} aria-label="Записать перевод «{term?.name}»" />
        <div class="name">
          <strong>{term?.name ?? item.id}</strong>
          <span>Сейчас: {term?.translation ?? "нет перевода"}</span>
        </div>
        <input class="input" bind:value={item.translation} placeholder="Перевод" aria-label="Перевод «{term?.name}»" />
        <select
          class="input"
          value={item.category ?? term?.category}
          onchange={(event) => (item.category = event.currentTarget.value as TermCategory)}
          aria-label="Категория «{term?.name}»"
        >
          {#each categories as category (category.value)}<option value={category.value}>{category.label}</option>{/each}
        </select>
      </div>
    {/each}
  </div>
  <div class="pick-actions">
    <button class="btn small" disabled={writing} onclick={() => run.discard()}>Отбросить</button>
    <button class="btn small primary" disabled={writing || run.chosen.length === 0} onclick={writeChosen}>
      {#if writing}<LoaderCircle size={13} class="spin" />{:else}<Check size={13} />{/if}
      <span>Записать выбранные · {run.chosen.length}</span>
    </button>
  </div>
{:else if filter === "trash"}
  <div class="list">
    {#each data.discardedTerms as item (item.key)}
      <div class="row">
        <div class="name">
          <strong>{item.text}</strong>
          <span>{item.givenToCharacter ? "Передано персонажу" : (item.translation ?? "Нет перевода")}</span>
        </div>
        <div class="row-tools">
          {#if confirmForget === item.key}
            <span class="confirm-text">Удалить окончательно?</span>
            <button class="btn small danger" onclick={() => act(() => commands.forgetTermName(projectId, item.key), () => (confirmForget = null))}>Да</button>
            <button class="btn small" onclick={() => (confirmForget = null)}>Нет</button>
          {:else}
            <button
              class="icon-button"
              aria-label="Вернуть в термины"
              title="Вернуть в термины с переводом, категорией, описанием и формами"
              onclick={() => act(() => commands.restoreTerm(projectId, item.key))}><Undo2 size={14} /></button
            >
            <button
              class="icon-button danger"
              aria-label="Удалить окончательно"
              title="Удалить окончательно: название забудется, модель может найти его снова"
              onclick={() => (confirmForget = item.key)}><Trash2 size={14} /></button
            >
          {/if}
        </div>
      </div>
    {:else}
      <div class="empty">
        <strong>Корзина пуста</strong>
        <span>Сюда уходят удалённые термины: модель не предложит их снова</span>
      </div>
    {/each}
  </div>
{:else}
  <div class="master with-lines">
    <div class="list-side">
      <input class="input" placeholder="Поиск по названию и переводу" bind:value={query} aria-label="Поиск терминов" />
      <div class="items">
        {#each shown as term (term.id)}
          <button class="item" class:active={term.id === draft?.id} onclick={() => (selected = term.id)}>
            <strong>{term.name}</strong>
            <span>{term.translation ?? "Нет перевода"} · {categoryLabel(term.category)}</span>
          </button>
        {:else}
          <p class="faint">{data.terms.length ? "Ничего не нашлось" : "Терминов пока нет: модель найдёт их, когда описывает сцену, или добавьте ниже"}</p>
        {/each}
      </div>
      <form class="add" onsubmit={(event) => (event.preventDefault(), create())}>
        <input class="input" placeholder="Новый термин" bind:value={newName} aria-label="Название нового термина" />
        <button
          class="icon-button"
          type="submit"
          disabled={creating || !newName.trim()}
          aria-label="Добавить термин"
          title="Добавить термин. Категория выбирается в записи"
        >
          {#if creating}<LoaderCircle size={14} class="spin" />{:else}<Plus size={14} />{/if}
        </button>
      </form>
    </div>

    {#if draft}
      <div class="editor" class:saving>
        <div class="editor-head">
          <strong title={draft.name}>{draft.name}</strong>
          <div class="editor-tools">
            <DescribeParts {projectId} owner={{ kind: "term", id: draft.id }} {runs} bind:confirming={resetting} onDescribed={described} {onError} />
            {#if !resetting}
              <button
                class="icon-button"
                class:active={giving}
                disabled={data.characters.length === 0}
                aria-label="Это персонаж"
                aria-pressed={giving}
                title="Это персонаж: имя или фамилия, название станет формой персонажа"
                onclick={() => (giving = !giving)}><UserRound size={14} /></button
              >
              <button
                class="icon-button danger"
                aria-label="В корзину"
                title="В корзину: модель не предложит термин снова, из корзины его можно вернуть"
                onclick={toTrash}><Trash2 size={14} /></button
              >
            {/if}
          </div>
        </div>

        {#if giving && !resetting}
          <Combobox
            text={giveText}
            options={characterOptions}
            label="Персонаж"
            placeholder="Кому отдать название"
            empty="Персонажей нет"
            oninput={(text) => (giveText = text)}
            onpick={(id) => draft && act(() => commands.giveTermToCharacter(projectId, draft!.id, Number(id)), forget)}
          />
        {/if}

        <div class="grid">
          <label class="field">Название в игре<input class="input" bind:value={draft.name} onchange={() => save()} /></label>
          <label class="field">Перевод<input class="input" bind:value={draft.translation} onchange={() => save()} /></label>
          <label class="field">
            Категория
            <select class="input" bind:value={draft.category} onchange={() => save()}>
              {#each categories as item (item.value)}<option value={item.value}>{item.label}</option>{/each}
            </select>
          </label>
        </div>

        <GlossaryForms forms={draft.forms} hint="Сокращения и короткие формы" onchange={(forms: NameForm[]) => save({ forms })} />
        <label class="field">Описание<textarea class="input" rows="6" bind:value={draft.description} onchange={() => save()}></textarea></label>

        {#if fieldError}<p class="error-text">{fieldError}</p>{/if}
      </div>
      <EntryLines
        subject={`term:${draft.id}`}
        load={() => commands.entryLines(projectId, { kind: "term", id: draft?.id ?? 0 })}
        empty="Название и формы в тексте не встречаются"
        revision={linesRevision}
        {onOpenLine}
        {onError}
      />
    {:else}
      <div class="empty">
        <strong>Термин не выбран</strong>
        <span>Выберите термин в списке</span>
      </div>
    {/if}
  </div>
{/if}
