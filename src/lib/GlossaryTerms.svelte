<script lang="ts">
  import { onMount } from "svelte";
  import { Check, Languages, LoaderCircle, Plus, Trash2, Undo2, UserRound } from "lucide-svelte";
  import Combobox from "./Combobox.svelte";
  import GlossaryForms from "./GlossaryForms.svelte";
  import SuggestStatus from "./SuggestStatus.svelte";
  import { commands, type TermCandidateDto as TermCandidate, type TermCategory } from "./bindings";
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
    reload,
    patch,
    onError,
  }: {
    projectId: string;
    data: Glossary;
    runs: GlossaryRuns;
    reload: () => Promise<void>;
    patch: (term: Term) => void;
    onError: (message: string) => void;
  } = $props();

  const run = $derived(runs.terms);
  const untranslated = $derived(data.terms.filter((term) => !term.translation?.trim()));
  let writing = $state(false);

  function runTitle() {
    if (untranslated.length === 0) return "У всех терминов есть перевод";
    if (runsBusy(runs)) return "Модель уже работает или ждёт отбор";
    return "Модель предложит перевод, категорию и описание терминам без перевода";
  }

  function startRun() {
    const ids = untranslated.map((term) => term.id);
    run.start((channel) => commands.suggestTerms(projectId, ids, null, channel), onError);
  }

  /** Отмеченное записывается целиком через `save_term`: термин подтверждается, своё описание остаётся. */
  async function writeChosen() {
    const chosen = run.chosen.filter((item) => item.translation.trim());
    writing = true;
    try {
      for (const item of chosen) {
        const term = data.terms.find((known) => known.id === item.id);
        if (!term) continue;
        await commands.saveTerm(projectId, {
          ...$state.snapshot(term),
          translation: item.translation.trim(),
          category: item.category ?? term.category,
          description: term.description ?? (item.description?.trim() || null),
          guess: false,
        });
      }
      run.finish(`Записано терминов: ${chosen.length}`);
      await reload();
    } catch (e) {
      onError(String(e));
    } finally {
      writing = false;
    }
  }

  type Filter = "all" | "fresh" | "guess" | "trash";
  let filter = $state<Filter>("all");

  /** Сколько «Новых» показывать сразу: в большой игре их тысячи, остальное находит поиск. */
  const FRESH_SHOWN = 300;
  /** «Новые термины» из текста игры. `null`: ещё не загружены. */
  let fresh = $state<TermCandidate[] | null>(null);
  let freshBusy = $state<string | null>(null);
  const freshShown = $derived((fresh ?? []).filter((item) => matches(query, item.text, item.example)));

  onMount(loadFresh);

  async function loadFresh() {
    try {
      fresh = await commands.newTerms(projectId);
    } catch (e) {
      onError(String(e));
    }
  }

  /** «Термин» или «Не термин»: название уходит из «Новых». */
  async function settle(text: string, work: () => Promise<unknown>) {
    freshBusy = text;
    try {
      await work();
      await reload();
      await loadFresh();
    } catch (e) {
      onError(String(e));
    } finally {
      freshBusy = null;
    }
  }
  let query = $state("");
  let selected = $state<number | null>(null);
  let newName = $state("");
  let newCategory = $state<TermCategory>("general");
  let creating = $state(false);
  let saving = $state(false);
  let fieldError = $state("");
  let confirmDelete = $state(false);
  let giving = $state(false);
  let giveText = $state("");
  let draft = $state<Term | null>(null);

  const guesses = $derived(data.terms.filter((term) => term.guess).length);
  const shown = $derived(
    data.terms
      .filter((term) => filter !== "guess" || term.guess)
      .filter((term) => matches(query, term.name, term.translation, ...term.forms.map((form) => form.text))),
  );

  $effect.pre(() => {
    const id = selected ?? shown[0]?.id ?? null;
    if (draft?.id === id) return;
    const found = data.terms.find((term) => term.id === id);
    draft = found ? structuredClone($state.snapshot(found)) : null;
    fieldError = "";
    confirmDelete = false;
    giving = false;
  });

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
      await loadFresh();
    } catch (e) {
      onError(String(e));
    }
  }

  async function create() {
    const name = newName.trim();
    if (!name) return;
    creating = true;
    try {
      const id = await commands.takeTerm(projectId, name, newCategory);
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
</script>

<div class="toolbar">
  <div class="toolbar-title">
    <strong>Термины</strong>
    <span>Места, организации, предметы и звания. Предположения модели идут в запросы с пометкой, пока их не подтвердят</span>
  </div>
  <div class="segmented" role="tablist" aria-label="Какие термины">
    <button role="tab" aria-selected={filter === "all"} class:on={filter === "all"} onclick={() => (filter = "all")}>Все · {data.terms.length}</button>
    <button
      role="tab"
      aria-selected={filter === "fresh"}
      class:on={filter === "fresh"}
      title="Названия из текста игры, которых нет в глоссарии"
      onclick={() => (filter = "fresh")}>Новые · {fresh?.length ?? "…"}</button
    >
    <button role="tab" aria-selected={filter === "guess"} class:on={filter === "guess"} onclick={() => (filter = "guess")}>Модель · {guesses}</button>
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

{#if run.review}
  <div class="list">
    <p class="pick-head">Предложения модели. Отмеченные запишутся в глоссарий, перевод и описание можно поправить</p>
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
        {#if term?.description}
          <span class="faint" title={term.description}>Своё описание остаётся</span>
        {:else}
          <input class="input" bind:value={item.description} placeholder="Описание" aria-label="Описание «{term?.name}»" />
        {/if}
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
{:else if filter === "fresh"}
  <div class="list">
    <div class="fresh-head">
      <input class="input" placeholder="Поиск по названию и реплике" bind:value={query} aria-label="Поиск новых терминов" />
      <span class="faint">Названия с заглавной буквы и сокращения из реплик игры. Среди них бывает мусор: его отправляют в корзину</span>
    </div>
    {#if fresh === null}
      <div class="empty"><LoaderCircle size={20} class="spin" /><span>Поиск названий в тексте игры</span></div>
    {:else}
      {#each freshShown.slice(0, FRESH_SHOWN) as item (item.text)}
        <div class="row fresh" class:busy={freshBusy === item.text}>
          <div class="name">
            <strong>{item.text}</strong>
            <span title={item.example}>реплик: {item.occurrences} · {item.example}</span>
          </div>
          <div class="fresh-actions">
            <button class="btn small" disabled={freshBusy !== null} title="Взять в глоссарий: перевод и категорию можно задать потом" onclick={() => settle(item.text, () => commands.takeTerm(projectId, item.text, "general"))}>
              <Plus size={13} />
              <span>Термин</span>
            </button>
            <button class="btn small" disabled={freshBusy !== null} title="Не термин: убрать в корзину и не предлагать снова" onclick={() => settle(item.text, () => commands.discardTermName(projectId, item.text))}>
              <Trash2 size={13} />
              <span>Не термин</span>
            </button>
          </div>
        </div>
      {:else}
        <div class="empty">
          <strong>{fresh.length ? "Ничего не нашлось" : "Новых названий нет"}</strong>
          <span>{fresh.length ? "Уточните поиск" : "Все названия из текста игры уже в глоссарии или в корзине"}</span>
        </div>
      {/each}
      {#if freshShown.length > FRESH_SHOWN}
        <p class="faint more">Показаны {FRESH_SHOWN} самых частых из {freshShown.length}: остальные находит поиск</p>
      {/if}
    {/if}
  </div>
{:else if filter === "trash"}
  <div class="list">
    {#each data.discardedTerms as item (item.key)}
      <div class="row">
        <div class="name">
          <strong>{item.text}</strong>
          <span>{item.givenToCharacter ? "Передано персонажу" : "Не термин"}</span>
        </div>
        <button class="btn small" onclick={() => act(() => commands.restoreTermName(projectId, item.key))}>
          <Undo2 size={13} />
          <span>Вернуть</span>
        </button>
      </div>
    {:else}
      <div class="empty">
        <strong>Корзина пуста</strong>
        <span>Сюда уходят названия, которые не термины: модель не предложит их снова</span>
      </div>
    {/each}
  </div>
{:else}
  <div class="master">
    <div class="list-side">
      <input class="input" placeholder="Поиск по названию и переводу" bind:value={query} aria-label="Поиск терминов" />
      <div class="items">
        {#each shown as term (term.id)}
          <button class="item" class:active={term.id === draft?.id} onclick={() => (selected = term.id)}>
            <strong>{term.name}{#if term.guess}<span class="guess">Модель</span>{/if}</strong>
            <span>{term.translation ?? "Нет перевода"} · {categoryLabel(term.category)}</span>
          </button>
        {:else}
          <p class="faint">{data.terms.length ? "Ничего не нашлось" : "Терминов пока нет: модель предложит их при переводе, или добавьте ниже"}</p>
        {/each}
      </div>
      <form class="add stacked" onsubmit={(event) => (event.preventDefault(), create())}>
        <input class="input name" placeholder="Название нового термина" bind:value={newName} aria-label="Название нового термина" />
        <select class="input category" bind:value={newCategory} aria-label="Категория нового термина">
          {#each categories as item (item.value)}<option value={item.value}>{item.label}</option>{/each}
        </select>
        <button class="btn small" type="submit" disabled={creating || !newName.trim()}>
          {#if creating}<LoaderCircle size={12} class="spin" />{:else}<Plus size={12} />{/if}
          <span>Добавить</span>
        </button>
      </form>
    </div>

    {#if draft}
      <div class="card editor" class:saving>
        <div class="card-head">
          <strong>{draft.name}</strong>
          {#if draft.guess}
            <button class="btn small warning" title="Перевод проверен: в запросы без пометки" onclick={() => save({ guess: false })}>
              <Check size={13} />
              <span>Подтвердить</span>
            </button>
          {/if}
          {#if confirmDelete}
            <span class="confirm-text">Удалить?</span>
            <button class="btn small danger" onclick={() => draft && act(() => commands.deleteTerm(projectId, draft!.id), forget)}>Да</button>
            <button class="btn small" onclick={() => (confirmDelete = false)}>Нет</button>
          {:else}
            <button class="icon-button danger" aria-label="Удалить термин" title="Удалить термин" onclick={() => (confirmDelete = true)}>
              <Trash2 size={14} />
            </button>
          {/if}
        </div>

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
        <label class="field">Описание<textarea class="input" rows="4" bind:value={draft.description} onchange={() => save()}></textarea></label>

        <div class="card-actions">
          {#if giving}
            <div class="picker">
              <Combobox
                text={giveText}
                options={characterOptions}
                label="Персонаж"
                placeholder="Кому отдать название"
                empty="Персонажей нет"
                oninput={(text) => (giveText = text)}
                onpick={(id) => draft && act(() => commands.giveTermToCharacter(projectId, draft!.id, Number(id)), forget)}
              />
            </div>
          {/if}
          <button
            class="btn small"
            disabled={data.characters.length === 0}
            title="Это имя или фамилия: название станет формой персонажа"
            onclick={() => (giving = !giving)}
          >
            <UserRound size={13} />
            <span>Это персонаж</span>
          </button>
          <button
            class="btn small"
            title="Не термин: убрать и не предлагать снова"
            onclick={() =>
              draft &&
              act(async () => {
                await commands.deleteTerm(projectId, draft!.id);
                await commands.discardTermName(projectId, draft!.name);
              }, forget)}
          >
            <Trash2 size={13} />
            <span>Не термин</span>
          </button>
        </div>

        {#if fieldError}<p class="error-text">{fieldError}</p>{/if}
      </div>
    {:else}
      <div class="empty">
        <strong>Термин не выбран</strong>
        <span>Выберите термин в списке</span>
      </div>
    {/if}
  </div>
{/if}

<style>
  .fresh-head {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: 4px 4px 8px;
  }

  .fresh-head .input {
    max-width: 360px;
  }

  .row.fresh.busy {
    opacity: 0.55;
  }

  .row.fresh .name {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .row.fresh .name strong {
    color: var(--text);
    font-size: 12px;
    font-weight: 600;
  }

  .row.fresh .name span {
    overflow: hidden;
    color: var(--text-faint);
    font-size: 11px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .fresh-actions {
    display: flex;
    flex-shrink: 0;
    gap: 6px;
  }

  .more {
    margin: 10px 4px 0;
  }
</style>
