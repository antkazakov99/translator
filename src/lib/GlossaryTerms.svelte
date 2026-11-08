<script lang="ts">
  import { Check, LoaderCircle, Plus, Trash2, Undo2, UserRound } from "lucide-svelte";
  import Combobox from "./Combobox.svelte";
  import GlossaryForms from "./GlossaryForms.svelte";
  import { commands, type TermCategory } from "./bindings";
  import { categories, categoryLabel, characterName, matches, type Glossary, type NameForm, type Term } from "./glossary";

  let {
    projectId,
    data,
    reload,
    patch,
    onError,
  }: {
    projectId: string;
    data: Glossary;
    reload: () => Promise<void>;
    patch: (term: Term) => void;
    onError: (message: string) => void;
  } = $props();

  type Filter = "all" | "guess" | "trash";
  let filter = $state<Filter>("all");
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
    <button role="tab" aria-selected={filter === "guess"} class:on={filter === "guess"} onclick={() => (filter = "guess")}>Модель · {guesses}</button>
    <button role="tab" aria-selected={filter === "trash"} class:on={filter === "trash"} onclick={() => (filter = "trash")}>
      Корзина · {data.discardedTerms.length}
    </button>
  </div>
</div>

{#if filter === "trash"}
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
