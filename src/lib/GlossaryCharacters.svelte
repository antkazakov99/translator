<script lang="ts">
  import { Check, Languages, LoaderCircle, Plus, Trash2, TriangleAlert } from "lucide-svelte";
  import DescribeParts from "./DescribeParts.svelte";
  import EntryLines from "./EntryLines.svelte";
  import GlossaryForms from "./GlossaryForms.svelte";
  import NamePicks from "./NamePicks.svelte";
  import SuggestStatus from "./SuggestStatus.svelte";
  import { commands, type EntryLineDto_Serialize as EntryLine } from "./bindings";
  import {
    characterName,
    characterTranslation,
    genders,
    languageList,
    matches,
    runsBusy,
    speakerName,
    untranslatedNames,
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
    onOpenLine,
    onError,
  }: {
    projectId: string;
    data: Glossary;
    runs: GlossaryRuns;
    selected: number | null;
    reload: () => Promise<void>;
    /** Сохранённый персонаж без перечитывания глоссария: правка в соседнем поле не теряется. */
    patch: (character: Character) => void;
    /** Реплика персонажа в таблице перевода или в исходнике. */
    onOpenLine: (line: EntryLine, where: "table" | "source") => void;
    onError: (message: string) => void;
  } = $props();

  const run = $derived(runs.names);
  /** Имена, фамилии и формы без перевода у всех персонажей. */
  const untranslated = $derived(data.characters.reduce((sum, character) => sum + untranslatedNames(character), 0));
  let writing = $state(false);

  function runTitle() {
    if (untranslated === 0) return "У всех имён, фамилий и форм есть перевод";
    if (runsBusy(runs)) return "Модель уже работает или ждёт отбор";
    return "Модель определит происхождение имён, напишет имена и фамилии без перевода по правилам их языков и проверит написания. Формы пишутся от записанного имени: у персонажа без перевода имени их спросит следующий запуск";
  }

  /** Формы, которые ждут записи имени владельца: после записи отбора их спросит следующий запуск. */
  let waitingForms = $state(0);

  function startRun() {
    const ids = data.characters.filter((character) => untranslatedNames(character) > 0).map((character) => character.id);
    run.start(async (channel) => {
      const result = await commands.suggestNames(projectId, ids, null, channel);
      waitingForms = result.waitingForms;
      const warnings: string[] = [];
      if (result.missingRules.length > 0) {
        warnings.push(
          `Нет правил для языков: ${languageList(result.missingRules)}. Имена этих языков написаны без них: правила можно добавить в «Правилах имён» и перевести заново`,
        );
      }
      if (result.waitingForms > 0) {
        warnings.push(`Форм без перевода: ${result.waitingForms}. Они пишутся от записанного имени: запишите отбор и запустите перевод ещё раз`);
      }
      run.warning = warnings.join(". ");
      return result;
    }, onError);
  }

  /** Отмеченное пишет `write_names`, та же запись, что у автоперевода: написания ложатся в пустые поля. */
  async function writeChosen() {
    const picks = run.chosen
      .filter((item) => item.translation.trim())
      .map((item) => ({ character: item.character, field: item.field, source: item.source, translation: item.translation.trim() }));
    writing = true;
    try {
      const written = await commands.writeNames(projectId, picks);
      run.finish(waitingForms > 0 ? `Записано персонажей: ${written}. Теперь можно перевести формы: ${waitingForms}` : `Записано персонажей: ${written}`);
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
  /** Открыт вопрос о сбросе заметок модели: остальные действия шапки на это время скрыты. */
  let resetting = $state(false);
  /** Карточка в правке. Сбрасывается только при выборе другого персонажа. */
  let draft = $state<Character | null>(null);

  const shown = $derived(
    data.characters.filter((character) =>
      matches(query, characterName(character), characterTranslation(character), ...character.forms.map((form) => form.text)),
    ),
  );
  const speakers = $derived(draft ? data.speakers.filter((speaker) => speaker.characters.includes(draft!.id)) : []);
  /** Сохранённые имена, формы и говорящие: по ним ищутся реплики персонажа. */
  const linesRevision = $derived.by(() => {
    const saved = data.characters.find((character) => character.id === draft?.id);
    return JSON.stringify([saved?.firstName, saved?.lastName, saved?.forms.map((form) => form.text), speakers.map((speaker) => speaker.key)]);
  });

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
<SuggestStatus run={runs.entries} {onError} />

{#if run.review}
  {#if run.warning}
    <div class="banner warn page-error" role="status">
      <TriangleAlert size={14} />
      <span>{run.warning}</span>
    </div>
  {/if}
  <div class="list">
    <p class="pick-head">Написания от модели. Отмеченные запишутся в пустые поля персонажей, написание можно выбрать из вариантов или поправить</p>
    <NamePicks items={run.review} {owner} />
  </div>
  <div class="pick-actions">
    <button class="btn small" disabled={writing} onclick={() => run.discard()}>Отбросить</button>
    <button class="btn small primary" disabled={writing || run.chosen.length === 0} onclick={writeChosen}>
      {#if writing}<LoaderCircle size={13} class="spin" />{:else}<Check size={13} />{/if}
      <span>Записать выбранные · {run.chosen.length}</span>
    </button>
  </div>
{:else}
<div class="master with-lines">
  <div class="list-side">
    <input class="input" placeholder="Поиск по имени и формам" bind:value={query} aria-label="Поиск персонажей" />
    <div class="items">
      {#each shown as character (character.id)}
        <button class="item" class:active={character.id === draft?.id} onclick={() => (selected = character.id)}>
          <strong>{characterName(character)}</strong>
          <span>{characterTranslation(character) || "Нет перевода"}</span>
        </button>
      {:else}
        <p class="faint">{data.characters.length ? "Никого не нашлось" : "Персонажей пока нет: возьмите их из «Говорящих» или добавьте ниже"}</p>
      {/each}
    </div>
    <form class="add" onsubmit={(event) => (event.preventDefault(), create())}>
      <input class="input" placeholder="Новый персонаж" bind:value={newName} aria-label="Имя нового персонажа" />
      <button
        class="icon-button"
        type="submit"
        disabled={creating || !newName.trim()}
        aria-label="Добавить персонажа"
        title="Добавить персонажа, которого только упоминают"
      >
        {#if creating}<LoaderCircle size={14} class="spin" />{:else}<Plus size={14} />{/if}
      </button>
    </form>
  </div>

  {#if draft}
    <div class="editor" class:saving>
      <div class="editor-head">
        <strong title={characterName(draft)}>{characterName(draft)}</strong>
        <div class="editor-tools">
          {#if confirmDelete}
            <span class="confirm-text">Удалить?</span>
            <button class="btn small danger" onclick={remove}>Да</button>
            <button class="btn small" onclick={() => (confirmDelete = false)}>Нет</button>
          {:else}
            <DescribeParts {projectId} owner={{ kind: "character", id: draft.id }} {runs} bind:confirming={resetting} onDescribed={described} {onError} />
            {#if !resetting}
              <button class="icon-button danger" aria-label="Удалить персонажа" title="Удалить персонажа" onclick={() => (confirmDelete = true)}>
                <Trash2 size={14} />
              </button>
            {/if}
          {/if}
        </div>
      </div>

      <!-- Связи стоят сразу под шапкой, как персонажи у говорящего. -->
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
      <label class="field">Описание<textarea class="input" rows="6" bind:value={draft.description} onchange={save}></textarea></label>

      {#if fieldError}<p class="error-text">{fieldError}</p>{/if}
    </div>
    <EntryLines
      subject={`character:${draft.id}`}
      load={() => commands.entryLines(projectId, { kind: "character", id: draft?.id ?? 0 })}
      mentions
      empty="Говорящих нет, а имя и формы в тексте не встречаются"
      revision={linesRevision}
      {onOpenLine}
      {onError}
    />
  {:else}
    <div class="empty">
      <strong>Персонаж не выбран</strong>
      <span>Выберите персонажа в списке</span>
    </div>
  {/if}
</div>
{/if}
