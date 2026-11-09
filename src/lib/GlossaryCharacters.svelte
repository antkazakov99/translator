<script lang="ts">
  import { Check, Languages, LoaderCircle, Sparkles, Plus, Trash2, TriangleAlert } from "lucide-svelte";
  import DescribeParts from "./DescribeParts.svelte";
  import GlossaryForms from "./GlossaryForms.svelte";
  import SuggestStatus from "./SuggestStatus.svelte";
  import { commands } from "./bindings";
  import {
    characterName,
    characterTranslation,
    genders,
    languageList,
    matches,
    nameFieldLabels,
    runsBusy,
    speakerName,
    untranslatedNames,
    withSpelledNames,
    type Character,
    type Glossary,
    type GlossaryRuns,
    type NameForm,
  } from "./glossary";

  let {
    projectId,
    data,
    runs,
    selected = $bindable(null),
    reload,
    patch,
    onError,
  }: {
    projectId: string;
    data: Glossary;
    runs: GlossaryRuns;
    selected: number | null;
    reload: () => Promise<void>;
    /** Сохранённый персонаж без перечитывания глоссария: правка в соседнем поле не теряется. */
    patch: (character: Character) => void;
    onError: (message: string) => void;
  } = $props();

  const run = $derived(runs.names);
  /** Имена, фамилии и формы без перевода у всех персонажей. */
  const untranslated = $derived(data.characters.reduce((sum, character) => sum + untranslatedNames(character), 0));
  let writing = $state(false);

  function runTitle() {
    if (untranslated === 0) return "У всех имён, фамилий и форм есть перевод";
    if (runsBusy(runs)) return "Модель уже работает или ждёт отбор";
    return "Модель определит происхождение имён, напишет имена, фамилии и формы без перевода по правилам их языков и проверит написания";
  }

  function startRun() {
    const ids = data.characters.filter((character) => untranslatedNames(character) > 0).map((character) => character.id);
    run.start(async (channel) => {
      const result = await commands.suggestNames(projectId, ids, null, channel);
      if (result.missingRules.length > 0) {
        run.warning = `Нет правил для языков: ${languageList(result.missingRules)}. Имена этих языков написаны без них: правила можно добавить в «Правилах имён» и перевести заново`;
      }
      return result;
    }, onError);
  }

  /** Отмеченное ложится в пустые поля карточки и записывается персонажем целиком. */
  async function writeChosen() {
    const chosen = run.chosen.filter((item) => item.translation.trim());
    writing = true;
    try {
      let written = 0;
      for (const id of new Set(chosen.map((item) => item.character))) {
        const character = data.characters.find((known) => known.id === id);
        const next = character && withSpelledNames($state.snapshot(character), chosen.filter((item) => item.character === id));
        if (!next) continue;
        await commands.saveCharacter(projectId, next);
        written += 1;
      }
      run.finish(`Записано персонажей: ${written}`);
      await reload();
    } catch (e) {
      onError(String(e));
    } finally {
      writing = false;
    }
  }

  function owner(id: number) {
    const character = data.characters.find((known) => known.id === id);
    return character ? characterName(character) : "Удалённый персонаж";
  }

  let query = $state("");
  let newName = $state("");
  let creating = $state(false);
  let saving = $state(false);
  let fieldError = $state("");
  let confirmDelete = $state(false);
  /** Карточка в правке. Сбрасывается только при выборе другого персонажа. */
  let draft = $state<Character | null>(null);

  /** Отмеченные для описания моделью. По умолчанию ничего: чтение сцен дорого. */
  let marked = $state<number[]>([]);
  const entriesRun = $derived(runs.entries);

  function toggleMark(id: number, on: boolean) {
    marked = on ? [...marked, id] : marked.filter((item) => item !== id);
  }

  function markShown() {
    marked = [...new Set([...marked, ...shown.map((item) => item.id)])];
  }

  function describeTitle() {
    if (marked.length === 0) return "Отметьте персонажей в списке";
    if (runsBusy(runs)) return "Модель уже работает или ждёт отбор";
    return "Модель прочитает части сцен каждого персонажа и напишет описание. Описанные без новых частей пропускаются";
  }

  async function describeMarked() {
    const owners = marked.map((id) => ({ kind: "character" as const, id }));
    const written = await entriesRun.start(
      (channel) => commands.describeEntries(projectId, owners, null, channel),
      onError,
      (count, stopped) => {
        const text = `Описано: ${count}`;
        return stopped ? `Остановлено. ${text}` : text;
      },
    );
    const done = new Set(written);
    marked = marked.filter((id) => !done.has(`character:${id}`));
    if (written.length > 0) await reload();
  }

  const shown = $derived(
    data.characters.filter((character) =>
      matches(query, characterName(character), characterTranslation(character), ...character.forms.map((form) => form.text)),
    ),
  );
  const speakers = $derived(draft ? data.speakers.filter((speaker) => speaker.characters.includes(draft!.id)) : []);

  $effect.pre(() => {
    const id = selected ?? data.characters[0]?.id ?? null;
    if (draft?.id === id) return;
    const found = data.characters.find((character) => character.id === id);
    draft = found ? structuredClone($state.snapshot(found)) : null;
    fieldError = "";
    confirmDelete = false;
  });

  /** Модель записала описание и манеру речи: они ложатся в открытую карточку. */
  async function described() {
    await reload();
    const found = data.characters.find((character) => character.id === draft?.id);
    if (draft && found) {
      draft.description = found.description;
      draft.speech = found.speech;
    }
  }

  async function save() {
    if (!draft) return;
    const snapshot = $state.snapshot(draft);
    saving = true;
    try {
      await commands.saveCharacter(projectId, snapshot);
      patch(snapshot);
      fieldError = "";
    } catch (e) {
      fieldError = String(e);
    } finally {
      saving = false;
    }
  }

  function setForms(forms: NameForm[]) {
    if (!draft) return;
    draft.forms = forms;
    save();
  }

  async function create() {
    const name = newName.trim();
    if (!name) return;
    creating = true;
    try {
      const id = await commands.createCharacter(projectId, name);
      newName = "";
      await reload();
      selected = id;
    } catch (e) {
      onError(String(e));
    } finally {
      creating = false;
    }
  }

  async function remove() {
    if (!draft) return;
    try {
      await commands.deleteCharacter(projectId, draft.id);
      selected = null;
      draft = null;
      await reload();
    } catch (e) {
      onError(String(e));
    }
  }
</script>

<div class="toolbar">
  <div class="toolbar-title">
    <strong>Персонажи</strong>
    <span>Имена с переводом и родом: модель видит их в каждой реплике, где они встречаются</span>
  </div>
  <button class="btn small primary" disabled={untranslated === 0 || runsBusy(runs)} title={runTitle()} onclick={startRun}>
    {#if run.active}<LoaderCircle size={13} class="spin" />{:else}<Languages size={13} />{/if}
    <span>Перевести имена · {untranslated}</span>
  </button>
</div>

<SuggestStatus {run} {onError} />
{#if !draft}<SuggestStatus run={entriesRun} {onError} />{/if}

{#if run.review}
  {#if run.warning}
    <div class="banner warn page-error" role="status">
      <TriangleAlert size={14} />
      <span>{run.warning}</span>
    </div>
  {/if}
  <div class="list">
    <p class="pick-head">Написания от модели. Отмеченные запишутся в пустые поля персонажей, написание можно выбрать из вариантов или поправить</p>
    {#each run.review as item, index (index)}
      {@const variant = item.variants.find((variant) => variant.translation === item.translation.trim())}
      <div class="pick-row names" class:off={!item.selected}>
        <input type="checkbox" bind:checked={item.selected} aria-label="Записать написание «{item.source}»" />
        <div class="name">
          <strong>{item.source}</strong>
          <span>{nameFieldLabels[item.field]} · {owner(item.character)}{item.languages.length ? ` · ${languageList(item.languages)}` : ""}</span>
          {#each item.issues as issue, issueIndex (issueIndex)}<span class="issue">{issue}</span>{/each}
        </div>
        <input class="input" bind:value={item.translation} aria-label="Написание «{item.source}»" />
        <div class="variants">
          {#if item.variants.length > 1}
            <div class="segmented" role="radiogroup" aria-label="Варианты написания «{item.source}»">
              {#each item.variants as variant (variant.translation)}
                <button
                  role="radio"
                  aria-checked={item.translation.trim() === variant.translation}
                  class:on={item.translation.trim() === variant.translation}
                  title={variant.description ?? ""}
                  onclick={() => (item.translation = variant.translation)}>{variant.translation}</button
                >
              {/each}
            </div>
          {/if}
          <span class="faint">{variant ? (variant.description ?? "") : item.translation.trim() ? "Своё написание" : ""}</span>
        </div>
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
{:else}
<div class="master">
  <div class="list-side">
    <input class="input" placeholder="Поиск по имени и формам" bind:value={query} aria-label="Поиск персонажей" />
    <div class="marks">
      <button class="btn small" disabled={shown.length === 0} onclick={markShown}>Отметить показанных</button>
      <button class="btn small" disabled={marked.length === 0} onclick={() => (marked = [])}>Снять</button>
      <button class="btn small primary" disabled={marked.length === 0 || runsBusy(runs)} title={describeTitle()} onclick={describeMarked}>
        {#if entriesRun.active}<LoaderCircle size={12} class="spin" />{:else}<Sparkles size={12} />{/if}
        <span>Описать · {marked.length}</span>
      </button>
    </div>
    <div class="items">
      {#each shown as character (character.id)}
        <div class="marked-item">
          <input
            type="checkbox"
            checked={marked.includes(character.id)}
            onchange={(event) => toggleMark(character.id, event.currentTarget.checked)}
            aria-label="Отметить {characterName(character)}"
          />
          <button class="item" class:active={character.id === draft?.id} onclick={() => (selected = character.id)}>
            <strong>{characterName(character)}</strong>
            <span>{characterTranslation(character) || "Нет перевода"}</span>
          </button>
        </div>
      {:else}
        <p class="faint">{data.characters.length ? "Никого не нашлось" : "Персонажей пока нет: возьмите их из «Говорящих» или добавьте ниже"}</p>
      {/each}
    </div>
    <form class="add" onsubmit={(event) => (event.preventDefault(), create())}>
      <input class="input" placeholder="Имя персонажа" bind:value={newName} aria-label="Имя нового персонажа" />
      <button class="btn small" type="submit" disabled={creating || !newName.trim()} title="Персонаж, которого только упоминают">
        {#if creating}<LoaderCircle size={12} class="spin" />{:else}<Plus size={12} />{/if}
        <span>Добавить</span>
      </button>
    </form>
  </div>

  {#if draft}
    <div class="card editor" class:saving>
      <div class="card-head">
        <strong>{characterName(draft)}</strong>
        {#if confirmDelete}
          <span class="confirm-text">Удалить?</span>
          <button class="btn small danger" onclick={remove}>Да</button>
          <button class="btn small" onclick={() => (confirmDelete = false)}>Нет</button>
        {:else}
          <button class="icon-button danger" aria-label="Удалить персонажа" title="Удалить персонажа" onclick={() => (confirmDelete = true)}>
            <Trash2 size={14} />
          </button>
        {/if}
      </div>

      <div class="grid">
        <label class="field">Имя<input class="input" bind:value={draft.firstName} onchange={save} /></label>
        <label class="field">Перевод имени<input class="input" bind:value={draft.translatedFirstName} onchange={save} /></label>
        <label class="field">Фамилия<input class="input" bind:value={draft.lastName} onchange={save} /></label>
        <label class="field">Перевод фамилии<input class="input" bind:value={draft.translatedLastName} onchange={save} /></label>
      </div>

      <div class="field">
        <span>Род</span>
        <div class="segmented gender" role="radiogroup" aria-label="Род">
          {#each genders as item (item.label)}
            <button
              role="radio"
              aria-checked={draft.gender === item.value}
              class:on={draft.gender === item.value}
              onclick={() => {
                if (draft) draft.gender = item.value;
                save();
              }}>{item.label}</button
            >
          {/each}
        </div>
      </div>

      <GlossaryForms forms={draft.forms} hint="Обращения, прозвища, подписи из игры" onchange={setForms} />

      <label class="field">Манера речи<input class="input" bind:value={draft.speech} onchange={save} placeholder="Например: грубит, говорит коротко" /></label>
      <label class="field">Описание<textarea class="input" rows="4" bind:value={draft.description} onchange={save}></textarea></label>
      <DescribeParts {projectId} owner={{ kind: "character", id: draft.id }} {runs} onDescribed={described} {onError} />

      <div class="field">
        <span>Говорящие</span>
        <div class="badges">
          {#each speakers as speaker (speaker.key)}
            <span class="badge" class:accent={speaker.characters[0] === draft.id} title={speaker.key}>{speakerName(speaker)}</span>
          {:else}
            <span class="faint">Только упоминается: говорящих нет</span>
          {/each}
        </div>
      </div>

      {#if fieldError}<p class="error-text">{fieldError}</p>{/if}
    </div>
  {:else}
    <div class="empty">
      <strong>Персонаж не выбран</strong>
      <span>Выберите персонажа в списке</span>
    </div>
  {/if}
</div>
{/if}

<style>
  .marks {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .marked-item {
    display: flex;
    gap: 6px;
    align-items: center;
  }

  .marked-item .item {
    flex: 1;
    min-width: 0;
  }

  .variants {
    display: flex;
    flex-direction: column;
    gap: 4px;
    align-items: flex-start;
    min-width: 0;
  }

  .variants .segmented {
    flex-wrap: wrap;
  }

  .name .issue {
    color: var(--warning);
    white-space: normal;
  }
</style>
