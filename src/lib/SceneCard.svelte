<script lang="ts">
  import { ChevronDown, ChevronRight, Eraser, LoaderCircle, SlidersHorizontal, Sparkles, TriangleAlert, X } from "lucide-svelte";
  import { commands, type PartNoteDto_Serialize as PartNote, type SceneDto_Serialize as Scene } from "./bindings";

  let {
    projectId,
    scene,
    sceneKey,
    stale,
    busy,
    describing,
    fileScenes,
    depthOpen,
    onDescribe,
    onDescribeFile,
    onDepth,
    onSaved,
    onClose,
    onError,
  }: {
    projectId: string;
    /** Сцена с репликами: название и описание. */
    scene: Scene;
    sceneKey: string;
    /** Игра изменила текст сцены после того, как модель его прочитала. */
    stale: boolean;
    /** Модель занята другим запуском. */
    busy: boolean;
    describing: boolean;
    /** Сколько сцен в файле открытой сцены: их описывает «Описать сцены файла». */
    fileScenes: number;
    /** Открыта карточка глубины чтения. */
    depthOpen: boolean;
    onDescribe: (key: string) => void;
    onDescribeFile: () => void;
    onDepth: () => void;
    onSaved: (key: string, title: string | null, description: string | null) => void;
    /** Скрыть боковую панель. */
    onClose: () => void;
    onError: (message: string) => void;
  } = $props();

  let draftTitle = $state("");
  let draftDescription = $state("");
  let saving = $state(false);
  let notesOpen = $state(false);
  let notes = $state<PartNote[] | null>(null);
  let confirmReset = $state(false);

  /** Что последним пришло из базы: поля перезаписываются, только когда это меняется, а не на каждый перечит реплик. */
  let seen = { key: "", title: null as string | null, description: null as string | null };

  $effect(() => {
    const key = sceneKey;
    const title = scene.title ?? null;
    const description = scene.description ?? null;
    const other = key !== seen.key;
    if (other || title !== seen.title) draftTitle = title ?? "";
    if (other || description !== seen.description) {
      draftDescription = description ?? "";
      confirmReset = false;
      // Новое описание значит новые заметки: открытый список перечитывается.
      if (notesOpen) loadNotes(key);
      else notes = null;
    }
    seen = { key, title, description };
  });

  async function loadNotes(key: string) {
    try {
      const found = await commands.sceneNotes(projectId, key);
      if (sceneKey === key) notes = found;
    } catch (e) {
      onError(String(e));
    }
  }

  function toggleNotes() {
    notesOpen = !notesOpen;
    if (notesOpen && notes === null) loadNotes(sceneKey);
  }

  async function save() {
    const key = sceneKey;
    saving = true;
    try {
      await commands.saveSceneDescription(projectId, key, draftTitle, draftDescription);
      onSaved(key, draftTitle.trim() || null, draftDescription.trim() || null);
    } catch (e) {
      onError(String(e));
    } finally {
      saving = false;
    }
  }

  async function resetNotes() {
    const key = sceneKey;
    confirmReset = false;
    try {
      await commands.resetSceneNotes(projectId, key);
      await loadNotes(key);
    } catch (e) {
      onError(String(e));
    }
  }

  const read = $derived(notes?.filter((part) => part.note !== null).length ?? 0);
</script>

<div class="scene-card" class:saving>
  <div class="card-head">
    <button
      class="btn small"
      disabled={busy}
      title={busy ? "Модель уже работает" : "Модель прочитает всю сцену и заменит описание"}
      onclick={() => onDescribe(sceneKey)}
    >
      {#if describing}<LoaderCircle size={13} class="spin" />{:else}<Sparkles size={13} />{/if}
      <span>Описать</span>
    </button>
    <button
      class="btn small"
      disabled={busy || fileScenes < 2}
      title={busy ? "Модель уже работает" : "Модель прочитает часть каждой сцены файла с глубиной чтения проекта. Описанные сцены без новых частей пропускаются"}
      onclick={onDescribeFile}
    >
      <span>Сцены файла · {fileScenes}</span>
    </button>
    <button class="icon-button" class:active={depthOpen} aria-label="Глубина чтения" title="Сколько частей читает модель для описаний" onclick={onDepth}
      ><SlidersHorizontal size={14} /></button
    >
    <span class="head-gap"></span>
    <button class="icon-button" aria-label="Скрыть описание сцены" title="Скрыть описание сцены" onclick={onClose}><X size={14} /></button>
  </div>

  {#if stale}
    <div class="banner warn" role="status">
      <TriangleAlert size={14} />
      <span>Игра изменила текст сцены после того, как модель его прочитала. Опишите сцену заново или поправьте описание</span>
    </div>
  {/if}

  <div class="fields">
    <label class="field">Название<input class="input" bind:value={draftTitle} onchange={save} placeholder="Модель предложит при описании" /></label>
    <label class="field"
      >Описание<textarea
        class="input"
        rows="8"
        bind:value={draftDescription}
        onchange={save}
        placeholder="Что происходит, кто участвует, на «ты» или на «вы». Уходит модели в каждом запросе перевода сцены"
      ></textarea></label
    >
  </div>

  <div class="notes">
    <div class="notes-head">
      <button class="notes-toggle" aria-expanded={notesOpen} onclick={toggleNotes}>
        {#if notesOpen}<ChevronDown size={13} />{:else}<ChevronRight size={13} />{/if}
        <span>Заметки модели по частям{notes ? ` · прочитано ${read} из ${notes.length}` : ""}</span>
      </button>
      {#if notesOpen && read > 0}
        {#if confirmReset}
          <span class="faint">Сбросить заметки?</span>
          <button class="btn small danger" onclick={resetNotes}>Да</button>
          <button class="btn small" onclick={() => (confirmReset = false)}>Нет</button>
        {:else}
          <button class="btn small" title="Следующее описание прочитает сцену заново" onclick={() => (confirmReset = true)}>
            <Eraser size={12} />
            <span>Сбросить</span>
          </button>
        {/if}
      {/if}
    </div>
    {#if notesOpen}
      {#if notes === null}
        <span class="faint"><LoaderCircle size={12} class="spin" /></span>
      {:else}
        {#each notes as part, index (index)}
          <div class="part" class:off={part.note !== null && !part.useful}>
            <span class="faint">Часть {index + 1} из {notes.length}: {part.start}</span>
            {#if part.note}
              <p>{part.note}</p>
              {#if !part.useful}<span class="faint">Без сведений: в описание не идёт</span>{/if}
            {:else}
              <span class="faint">Ещё не прочитана</span>
            {/if}
          </div>
        {/each}
      {/if}
    {/if}
  </div>
</div>

<style>
  .scene-card {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .scene-card.saving {
    opacity: 0.8;
  }

  .card-head {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .head-gap {
    flex: 1;
  }

  .fields {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  textarea {
    resize: vertical;
  }

  .notes {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .notes-head {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .notes-toggle {
    display: inline-flex;
    flex: 1;
    align-items: center;
    gap: 4px;
    padding: 0;
    border: 0;
    background: none;
    color: var(--text-muted);
    font-size: 12px;
    text-align: left;
  }

  .notes-toggle:hover {
    color: var(--text);
  }

  .faint {
    color: var(--text-faint);
    font-size: 11px;
  }

  .part {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 8px 0;
    border-bottom: 1px solid var(--divider-row);
  }

  .part p {
    margin: 0;
    color: var(--text);
    font-size: 12px;
    line-height: 1.5;
  }

  .part.off p {
    color: var(--text-faint);
  }
</style>
