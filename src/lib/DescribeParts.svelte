<script lang="ts">
  import { untrack } from "svelte";
  import { Eraser, LoaderCircle, Sparkles, Undo2, X } from "lucide-svelte";
  import SuggestStatus from "./SuggestStatus.svelte";
  import { commands, type EntryPartsDto_Serialize as EntryParts, type OwnerDto } from "./bindings";
  import { runsBusy, type GlossaryRuns } from "./glossary";
  import { sceneLabel } from "./scene-graph";
  import { depthChanges } from "./reading-depth.svelte";

  /** Части сцен персонажа или термина с заметками модели: описание по ним, исключение заметок и сцен. */
  let {
    projectId,
    owner,
    runs,
    onDescribed,
    onError,
  }: {
    projectId: string;
    owner: OwnerDto;
    runs: GlossaryRuns;
    /** Описание записано: карточка перечитывает запись. */
    onDescribed: () => Promise<void>;
    onError: (message: string) => void;
  } = $props();

  const run = $derived(runs.entries);
  let parts = $state<EntryParts | null>(null);
  let loading = $state(false);
  let confirmReset = $state(false);

  /** Перечитывается при смене записи, а не при каждом новом объекте `owner` от карточки. */
  const ownerKey = $derived(`${owner.kind}:${owner.id}`);

  $effect(() => {
    ownerKey;
    // Новая глубина чтения меняет части следующего запуска.
    depthChanges.count;
    untrack(() => {
      confirmReset = false;
      parts = null;
      load(owner);
    });
  });

  async function load(current: OwnerDto = owner) {
    loading = true;
    try {
      const found = await commands.entryParts(projectId, current);
      if (current.kind === owner.kind && current.id === owner.id) parts = found;
    } catch (e) {
      onError(String(e));
    } finally {
      loading = false;
    }
  }

  const read = $derived(parts?.parts.filter((part) => part.note !== null).length ?? 0);

  async function describe() {
    const current = owner;
    const written = await run.start(
      (channel) => commands.describeEntry(projectId, current, null, channel),
      onError,
      (count, stopped) => {
        const text = count > 0 ? "Описание записано" : "Модель не нашла сведений для описания";
        return stopped ? `Остановлено. ${text}` : text;
      },
    );
    if (written.length > 0) await onDescribed();
    await load(current);
  }

  async function resetNotes() {
    confirmReset = false;
    try {
      await commands.resetEntryNotes(projectId, owner);
      await load();
    } catch (e) {
      onError(String(e));
    }
  }

  async function excludeNote(model: string, hash: string, excluded: boolean) {
    try {
      await commands.excludeNote(projectId, owner, model, hash, excluded);
      await load();
    } catch (e) {
      onError(String(e));
    }
  }

  async function excludeScene(key: string, excluded: boolean) {
    try {
      await commands.excludeScene(projectId, owner, key, excluded);
      await load();
    } catch (e) {
      onError(String(e));
    }
  }

  function describeTitle() {
    if (!parts || parts.parts.length === 0) return "Модели нечего читать: сцен нет или все исключены";
    if (runsBusy(runs)) return "Модель уже работает или ждёт отбор";
    return "Модель прочитает часть сцен, запишет заметки и заменит описание";
  }
</script>

<SuggestStatus {run} {onError} />

<div class="field describe">
  <div class="head">
    <span>Описание по сценам</span>
    {#if loading && !parts}<LoaderCircle size={12} class="spin" />{/if}
    {#if parts}
      <span class="faint">частей: {parts.parts.length} · прочитано: {read} · модель прочитает: {parts.picked}</span>
    {/if}
    <span class="spacer"></span>
    {#if parts && read > 0}
      {#if confirmReset}
        <span class="confirm-text">Сбросить заметки?</span>
        <button class="btn small danger" onclick={resetNotes}>Да</button>
        <button class="btn small" onclick={() => (confirmReset = false)}>Нет</button>
      {:else}
        <button class="btn small" title="Следующее описание прочитает сцены заново" onclick={() => (confirmReset = true)}>
          <Eraser size={12} />
          <span>Сбросить</span>
        </button>
      {/if}
    {/if}
    <button class="btn small primary" disabled={!parts || parts.parts.length === 0 || runsBusy(runs)} title={describeTitle()} onclick={describe}>
      {#if run.active}<LoaderCircle size={12} class="spin" />{:else}<Sparkles size={12} />{/if}
      <span>Описать</span>
    </button>
  </div>

  {#if parts && parts.excluded.length > 0}
    <div class="badges">
      <span class="faint">Исключены:</span>
      {#each parts.excluded as key (key)}
        <button class="badge" title="Вернуть сцену" onclick={() => excludeScene(key, false)}>
          {sceneLabel(key)}
          <Undo2 size={11} />
        </button>
      {/each}
    </div>
  {/if}

  {#if parts && parts.parts.length > 0}
    <details>
      <summary class="faint">Части и заметки</summary>
      {#each parts.parts as part, index (part.hash)}
        <div class="part" class:off={part.excluded || (part.note !== null && !part.useful)}>
          <div class="part-head">
            <span class="faint">Часть {index + 1}{part.picked ? "" : " · не в выборке"}</span>
            {#each part.scenes as key (key)}
              <span class="badge scene">
                {sceneLabel(key)}
                <button class="icon-button" aria-label="Исключить сцену {sceneLabel(key)}" title="Не давать сцену модели" onclick={() => excludeScene(key, true)}
                  ><X size={11} /></button
                >
              </span>
            {/each}
          </div>
          {#if part.note !== null}
            <p>{part.note}</p>
            {#if part.useful && part.model}
              <label class="faint toggle">
                <input type="checkbox" checked={!part.excluded} onchange={(event) => excludeNote(part.model ?? "", part.hash, !event.currentTarget.checked)} />
                В описание
              </label>
            {:else if !part.useful}
              <span class="faint">Без сведений: в описание не идёт</span>
            {/if}
          {:else}
            <span class="faint">{part.start}</span>
          {/if}
        </div>
      {/each}
    </details>
  {/if}
</div>

<style>
  .head {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
  }

  .spacer {
    flex: 1;
  }

  summary {
    cursor: pointer;
    padding: 4px 0;
  }

  .part {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 8px 0;
    border-bottom: 1px solid var(--divider-row);
  }

  .part.off p {
    color: var(--text-faint);
  }

  .part-head {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
  }

  .part p {
    margin: 0;
    color: var(--text);
    font-size: 12px;
    line-height: 1.5;
  }

  .badge.scene {
    display: inline-flex;
    gap: 2px;
    align-items: center;
  }

  button.badge {
    display: inline-flex;
    gap: 4px;
    align-items: center;
    cursor: pointer;
  }

  .toggle {
    display: flex;
    gap: 6px;
    align-items: center;
  }
</style>
