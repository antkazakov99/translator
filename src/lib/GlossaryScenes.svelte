<script lang="ts">
  import { onMount } from "svelte";
  import { Eraser, FileText, LoaderCircle, Sparkles, TriangleAlert } from "lucide-svelte";
  import SuggestStatus from "./SuggestStatus.svelte";
  import { commands, type PartNoteDto_Serialize as PartNote, type SceneEntryDto_Serialize as SceneEntry } from "./bindings";
  import { matches, runsBusy, type GlossaryRuns } from "./glossary";
  import { sceneLabel } from "./scene-graph";

  let {
    projectId,
    runs,
    onError,
  }: {
    projectId: string;
    runs: GlossaryRuns;
    onError: (message: string) => void;
  } = $props();

  /** Сколько сцен показывает список: остальные находятся поиском. */
  const SHOWN = 300;

  const run = $derived(runs.scenes);
  let scenes = $state<SceneEntry[] | null>(null);
  let selected = $state<string | null>(null);
  /** Отмеченные для группы. По умолчанию ничего: чтение всей игры дорого. */
  let marked = $state<string[]>([]);
  let query = $state("");
  let filter = $state<"all" | "empty" | "stale">("all");
  let notes = $state<PartNote[]>([]);
  let draftTitle = $state("");
  let draftDescription = $state("");
  let saving = $state(false);
  let confirmReset = $state(false);

  onMount(load);

  async function load() {
    try {
      scenes = await commands.sceneDescriptions(projectId);
    } catch (e) {
      onError(String(e));
    }
  }

  const filtered = $derived(
    (scenes ?? []).filter((scene) => {
      if (filter === "empty" && scene.description) return false;
      if (filter === "stale" && !scene.stale) return false;
      return matches(query, scene.key, scene.title, scene.description);
    }),
  );
  const shown = $derived(filtered.slice(0, SHOWN));
  const scene = $derived(scenes?.find((item) => item.key === selected) ?? null);
  const emptyCount = $derived((scenes ?? []).filter((item) => !item.description).length);
  const staleCount = $derived((scenes ?? []).filter((item) => item.stale).length);

  /** Список перечитывается после запуска и сброса: вместе с ним перечитываются и заметки выбранной сцены. */
  $effect(() => {
    const current = scene;
    draftTitle = current?.title ?? "";
    draftDescription = current?.description ?? "";
    confirmReset = false;
    if (current) loadNotes(current.key);
    else notes = [];
  });

  async function loadNotes(key: string) {
    try {
      const found = await commands.sceneNotes(projectId, key);
      if (selected === key) notes = found;
    } catch (e) {
      onError(String(e));
    }
  }

  function name(item: SceneEntry) {
    const label = sceneLabel(item.key);
    return item.title ? `${item.title} · ${label}` : label;
  }

  async function save() {
    if (!scene) return;
    const key = scene.key;
    saving = true;
    try {
      await commands.saveSceneDescription(projectId, key, draftTitle, draftDescription);
      if (scenes) {
        scenes = scenes.map((item) =>
          item.key === key ? { ...item, title: draftTitle.trim() || null, description: draftDescription.trim() || null } : item,
        );
      }
    } catch (e) {
      onError(String(e));
    } finally {
      saving = false;
    }
  }

  function toggle(key: string, on: boolean) {
    marked = on ? [...marked, key] : marked.filter((item) => item !== key);
  }

  function markShown() {
    const keys = new Set(marked);
    for (const item of shown) keys.add(item.key);
    marked = [...keys];
  }

  function describedNotice(written: number, stopped: boolean) {
    const text = `Описано сцен: ${written}`;
    return stopped ? `Остановлено. ${text}` : text;
  }

  async function describeOne() {
    if (!scene) return;
    const key = scene.key;
    await run.start((channel) => commands.describeScene(projectId, key, null, channel), onError, describedNotice);
    await load();
  }

  async function describeMarked() {
    const keys = [...marked];
    const written = await run.start((channel) => commands.describeScenes(projectId, keys, null, channel), onError, describedNotice);
    marked = marked.filter((key) => !written.includes(key));
    await load();
  }

  async function resetNotes() {
    if (!scene) return;
    const key = scene.key;
    confirmReset = false;
    try {
      await commands.resetSceneNotes(projectId, key);
      await load();
    } catch (e) {
      onError(String(e));
    }
  }

  function runTitle(count: number) {
    if (count === 0) return "Отметьте сцены в списке";
    if (runsBusy(runs)) return "Модель уже работает или ждёт отбор";
    return "Модель прочитает часть каждой сцены и напишет описание. Описанные сцены без новых частей пропускаются";
  }
</script>

<div class="toolbar">
  <div class="toolbar-title">
    <strong>Сцены</strong>
    <span>Описание сцены уходит модели в каждом запросе перевода её реплик: кто говорит, что происходит, на «ты» или на «вы»</span>
  </div>
  <button class="btn small primary" disabled={marked.length === 0 || runsBusy(runs)} title={runTitle(marked.length)} onclick={describeMarked}>
    {#if run.active}<LoaderCircle size={13} class="spin" />{:else}<Sparkles size={13} />{/if}
    <span>Описать отмеченные · {marked.length}</span>
  </button>
</div>

<SuggestStatus {run} {onError} />

{#if !scenes}
  <div class="empty"><LoaderCircle size={20} class="spin" /><span>Загрузка сцен</span></div>
{:else}
  <div class="master">
    <div class="list-side">
      <input class="input" placeholder="Поиск по метке, названию и описанию" bind:value={query} aria-label="Поиск сцен" />
      <div class="segmented" role="radiogroup" aria-label="Какие сцены показать">
        <button role="radio" aria-checked={filter === "all"} class:on={filter === "all"} onclick={() => (filter = "all")}>Все · {scenes.length}</button>
        <button role="radio" aria-checked={filter === "empty"} class:on={filter === "empty"} onclick={() => (filter = "empty")}
          >Без описания · {emptyCount}</button
        >
        <button role="radio" aria-checked={filter === "stale"} class:on={filter === "stale"} onclick={() => (filter = "stale")}
          >Устарело · {staleCount}</button
        >
      </div>
      <div class="marks">
        <button class="btn small" disabled={shown.length === 0} onclick={markShown}>Отметить показанные</button>
        <button class="btn small" disabled={marked.length === 0} onclick={() => (marked = [])}>Снять отметки</button>
      </div>
      <div class="items">
        {#each shown as item (item.key)}
          <div class="scene-item">
            <input
              type="checkbox"
              checked={marked.includes(item.key)}
              onchange={(event) => toggle(item.key, event.currentTarget.checked)}
              aria-label="Отметить сцену {sceneLabel(item.key)}"
            />
            <button class="item" class:active={item.key === selected} onclick={() => (selected = item.key)}>
              <strong>
                <span class="item-name">{name(item)}</span>
                {#if item.stale}<span class="guess">устарело</span>{/if}
              </strong>
              <span>{item.description ?? "Без описания"}</span>
            </button>
          </div>
        {:else}
          <p class="faint">{scenes.length ? "Ничего не нашлось" : "В снимке игры нет сцен"}</p>
        {/each}
        {#if filtered.length > shown.length}
          <p class="faint">Показаны первые {SHOWN} из {filtered.length}: остальные найдутся поиском</p>
        {/if}
      </div>
    </div>

    {#if scene}
      <div class="card editor" class:saving>
        <div class="card-head">
          <strong>{name(scene)}</strong>
          <button class="btn small primary" disabled={runsBusy(runs)} title={runsBusy(runs) ? "Модель уже работает или ждёт отбор" : "Модель прочитает всю сцену и заменит описание"} onclick={describeOne}>
            {#if run.active}<LoaderCircle size={13} class="spin" />{:else}<Sparkles size={13} />{/if}
            <span>Описать</span>
          </button>
        </div>
        <span class="faint"><FileText size={11} /> {scene.file ?? "Файл неизвестен"} · реплик: {scene.lines}</span>

        {#if scene.stale}
          <div class="banner warn" role="status">
            <TriangleAlert size={14} />
            <span>Игра изменила текст сцены после того, как модель его прочитала. Опишите сцену заново или поправьте описание</span>
          </div>
        {/if}

        <label class="field">Название<input class="input" bind:value={draftTitle} onchange={save} placeholder="Модель предложит при описании" /></label>
        <label class="field"
          >Описание<textarea class="input" rows="6" bind:value={draftDescription} onchange={save} placeholder="Что происходит, кто участвует, на «ты» или на «вы»"
          ></textarea></label
        >

        <div class="field">
          <div class="notes-head">
            <span>Заметки модели по частям</span>
            {#if scene.notes > 0}
              {#if confirmReset}
                <span class="confirm-text">Сбросить заметки?</span>
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
        </div>
      </div>
    {:else}
      <div class="empty">
        <strong>Сцена не выбрана</strong>
        <span>Выберите сцену в списке. Отметьте несколько, чтобы описать их одним запуском</span>
      </div>
    {/if}
  </div>
{/if}

<style>
  .marks {
    display: flex;
    gap: 6px;
  }

  .scene-item {
    display: flex;
    gap: 6px;
    align-items: center;
  }

  .scene-item .item {
    flex: 1;
    min-width: 0;
  }

  .item-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .notes-head {
    display: flex;
    gap: 8px;
    align-items: center;
    justify-content: space-between;
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
