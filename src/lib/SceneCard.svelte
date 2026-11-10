<script lang="ts">
  import { onMount } from "svelte";
  import { LoaderCircle, Sparkles, TriangleAlert } from "lucide-svelte";
  import RequestSlider from "./RequestSlider.svelte";
  import {
    commands,
    type PartNoteDto_Serialize as PartNote,
    type SceneDto_Serialize as Scene,
    type SceneEntriesDto_Serialize as SceneEntries,
  } from "./bindings";
  import { DEFAULT_READING_DEPTH, partsFor, type ReadingDepth } from "./reading-depth";

  let {
    projectId,
    scene,
    sceneKey,
    stale,
    busy,
    describing,
    onDescribe,
    onSaved,
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
    /** Описать сцену по `parts` частям. */
    onDescribe: (key: string, parts: number) => void;
    onSaved: (key: string, title: string | null, description: string | null) => void;
    onError: (message: string) => void;
  } = $props();

  let draftTitle = $state("");
  let draftDescription = $state("");
  let saving = $state(false);
  /** Части сцены: по ним считается слайдер. */
  let notes = $state<PartNote[] | null>(null);
  /** Персонажи и термины сцены. `null`: ещё не загружены. */
  let entries = $state<SceneEntries | null>(null);
  /** Глубина чтения приложения: по ней ставится число частей, пока его не сдвинули. */
  let depth = $state<ReadingDepth>(DEFAULT_READING_DEPTH);
  /** Сколько частей прочитает «Описать». `null`: по глубине чтения проекта. */
  let picked = $state<number | null>(null);

  onMount(async () => {
    try {
      depth = await commands.readingDepth();
    } catch (e) {
      onError(String(e));
    }
  });

  /** Что последним пришло из базы: поля перезаписываются, только когда это меняется, а не на каждый перечит реплик. */
  let seen = { key: "", title: null as string | null, description: null as string | null };

  $effect(() => {
    const key = sceneKey;
    const title = scene.title ?? null;
    const description = scene.description ?? null;
    const other = key !== seen.key;
    if (other || title !== seen.title) draftTitle = title ?? "";
    if (other) {
      picked = null;
      notes = null;
      entries = null;
    }
    if (other || description !== seen.description) {
      draftDescription = description ?? "";
      // Части нужны слайдеру. Новое описание значит запуск модели: он мог добавить персонажей и термины.
      loadNotes(key);
      loadEntries(key);
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

  /** Запуск описания кончился: он мог добавить персонажей и термины, даже не изменив описания. */
  let wasDescribing = false;
  $effect(() => {
    const now = describing;
    if (wasDescribing && !now) loadEntries(sceneKey);
    wasDescribing = now;
  });

  async function loadEntries(key: string) {
    try {
      const found = await commands.sceneEntries(projectId, key);
      if (sceneKey === key) entries = found;
    } catch (e) {
      onError(String(e));
    }
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

  const total = $derived(notes?.length ?? 0);
  const parts = $derived(Math.min(picked ?? partsFor(depth, total), total));
</script>

<div class="scene-card" class:saving>
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

  <div class="describe">
    {#if total > 1}
      <RequestSlider
        id="scene-parts-{sceneKey}"
        label="Читать частей"
        status="{parts} из {total}{picked === null ? ' · по глубине чтения' : ''}"
        max={total}
        value={parts}
        disabled={busy}
        onchange={(value) => (picked = value)}
      >
        {#snippet ends()}<span>1</span><span>все</span>{/snippet}
      </RequestSlider>
    {/if}
    <button
      class="btn small primary"
      disabled={busy || notes === null}
      title={busy
        ? "Модель уже работает"
        : "Модель найдёт в выбранных частях новых персонажей и термины и добавит их в глоссарий, опишет записи сцены без описания, потом заменит описание сцены. Название она пишет только в пустое поле"}
      onclick={() => onDescribe(sceneKey, Math.max(parts, 1))}
    >
      {#if describing}<LoaderCircle size={13} class="spin" />{:else}<Sparkles size={13} />{/if}
      <span>Описать моделью</span>
    </button>
  </div>

  <div class="entries" aria-label="Персонажи и термины сцены">
    {#if entries === null}
      <span class="faint"><LoaderCircle size={12} class="spin" /></span>
    {:else if entries.characters.length === 0 && entries.terms.length === 0}
      <span class="faint">Персонажей и терминов из глоссария в сцене нет. Модель найдёт новые, когда опишет сцену</span>
    {:else}
      {#each [{ title: "Персонажи", items: entries.characters }, { title: "Термины", items: entries.terms }] as group (group.title)}
        {#if group.items.length > 0}
          <div class="entry-group">
            <span class="group-title">{group.title} · {group.items.length}</span>
            {#each group.items as entry (entry.id)}
              <div class="entry" title={entry.description ?? "Без описания"}>
                <strong>{entry.name}</strong>
                <span class:missing={!entry.translation}>{entry.translation ?? "нет перевода"}</span>
                {#if !entry.description}<span class="faint">без описания</span>{/if}
              </div>
            {/each}
          </div>
        {/if}
      {/each}
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

  .describe {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 8px;
  }

  .describe > .btn {
    align-self: flex-start;
  }

  .fields {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  textarea {
    resize: vertical;
  }

  .faint {
    color: var(--text-faint);
    font-size: 11px;
  }

  .entries {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .entry-group {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .group-title {
    color: var(--text-muted);
    font-size: 12px;
    font-weight: 600;
  }

  .entry {
    display: flex;
    align-items: baseline;
    gap: 6px;
    padding: 4px 0;
    border-bottom: 1px solid var(--divider-row);
    font-size: 12px;
  }

  .entry strong {
    color: var(--text);
    font-weight: 600;
  }

  .entry span {
    overflow: hidden;
    color: var(--text-muted);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .entry span.missing {
    color: var(--text-faint);
  }
</style>
