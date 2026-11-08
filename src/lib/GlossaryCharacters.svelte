<script lang="ts">
  import { LoaderCircle, Plus, Trash2 } from "lucide-svelte";
  import GlossaryForms from "./GlossaryForms.svelte";
  import { commands } from "./bindings";
  import { characterName, characterTranslation, genders, matches, speakerName, type Character, type Glossary, type NameForm } from "./glossary";

  let {
    projectId,
    data,
    selected = $bindable(null),
    reload,
    patch,
    onError,
  }: {
    projectId: string;
    data: Glossary;
    selected: number | null;
    reload: () => Promise<void>;
    /** Сохранённый персонаж без перечитывания глоссария: правка в соседнем поле не теряется. */
    patch: (character: Character) => void;
    onError: (message: string) => void;
  } = $props();

  let query = $state("");
  let newName = $state("");
  let creating = $state(false);
  let saving = $state(false);
  let fieldError = $state("");
  let confirmDelete = $state(false);
  /** Карточка в правке. Сбрасывается только при выборе другого персонажа. */
  let draft = $state<Character | null>(null);

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

<div class="master">
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
