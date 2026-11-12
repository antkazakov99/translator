<script lang="ts">
  import { untrack } from "svelte";
  import { LoaderCircle } from "lucide-svelte";
  import type { EntryLineDto_Serialize as EntryLine } from "./bindings";
  import { matches } from "./glossary";
  import { sceneLabel } from "./scene-graph";

  /** Реплики записи глоссария по сюжету: где говорит персонаж или говорящий, где упомянут персонаж или термин. */
  let {
    subject,
    load,
    mentions = false,
    empty,
    revision = "",
    onOpenLine,
    onError,
  }: {
    /** Чьи это реплики, например `character:12`: со сменой записи сбрасываются поиск и фильтр. */
    subject: string;
    /** Читает реплики записи `subject`. */
    load: () => Promise<EntryLine[]>;
    /** Среди реплик бывают упоминания (у персонажа): есть фильтр «Говорит / Упоминания». */
    mentions?: boolean;
    /** Подсказка, когда реплик нет. */
    empty: string;
    /** Сохранённые имена и формы записи: их правка меняет найденные упоминания. */
    revision?: string;
    /** Реплика в таблице перевода или в исходнике. */
    onOpenLine: (line: EntryLine, where: "table" | "source") => void;
    onError: (message: string) => void;
  } = $props();

  type Filter = "all" | "spoken" | "mentioned";
  /** Сколько реплик показывать сразу: у главного героя их тысячи. */
  const PAGE = 200;

  let lines = $state<EntryLine[] | null>(null);
  let loading = $state(false);
  let filter = $state<Filter>("all");
  let query = $state("");
  let limit = $state(PAGE);

  $effect(() => {
    subject;
    untrack(() => {
      filter = "all";
      query = "";
      limit = PAGE;
      lines = null;
    });
  });

  $effect(() => {
    subject;
    revision;
    untrack(read);
  });

  async function read() {
    const current = subject;
    loading = true;
    try {
      const found = await load();
      // Пока шло чтение, открыли другую запись: её реплики придут своим чтением.
      if (current === subject) lines = found;
    } catch (e) {
      onError(String(e));
    } finally {
      loading = false;
    }
  }

  const spoken = $derived(lines?.filter((line) => line.spoken).length ?? 0);
  const sceneCount = $derived(new Set(lines?.map((line) => line.scene)).size);
  const shown = $derived(
    (lines ?? []).filter(
      (line) => (filter === "all" || line.spoken === (filter === "spoken")) && matches(query, line.line.sourceText, line.line.text),
    ),
  );

  /** Показанные реплики подряд по сценам. */
  const groups = $derived.by(() => {
    const result: { scene: string | null; title: string | null; lines: EntryLine[] }[] = [];
    for (const line of shown.slice(0, limit)) {
      const last = result.at(-1);
      if (last && last.scene === line.scene) last.lines.push(line);
      else result.push({ scene: line.scene, title: line.sceneTitle, lines: [line] });
    }
    return result;
  });

  function summary() {
    if (!lines || lines.length === 0) return "";
    // Число реплик стоит в заголовке панели: здесь только то, чего там нет.
    if (!mentions) return `сцен: ${sceneCount}`;
    return `сцен: ${sceneCount} · говорит: ${spoken} · упоминается: ${lines.length - spoken}`;
  }

  function sceneTitle(scene: string | null, title: string | null) {
    if (!scene) return "Вне сцен";
    return title ? `${title} · ${sceneLabel(scene)}` : sceneLabel(scene);
  }
</script>

<section class="entry-lines" aria-label="Реплики">
  <div class="head">
    <strong>Реплики{lines && lines.length > 0 ? ` · ${lines.length}` : ""}</strong>
    {#if loading && !lines}<LoaderCircle size={12} class="spin" />{/if}
    <span class="faint summary" title={summary()}>{summary()}</span>
  </div>

  {#if lines && lines.length === 0}
    <div class="empty">
      <strong>Реплик нет</strong>
      <span>{empty}</span>
    </div>
  {:else if lines}
    <div class="filters">
      <input class="input" type="search" bind:value={query} placeholder="Поиск по репликам" title="Поиск по оригиналу и переводу" aria-label="Поиск по репликам" />
      {#if mentions}
        <div class="segmented" role="radiogroup" aria-label="Какие реплики показать">
          <button role="radio" aria-checked={filter === "all"} class:on={filter === "all"} onclick={() => (filter = "all")}>Все</button>
          <button role="radio" aria-checked={filter === "spoken"} class:on={filter === "spoken"} onclick={() => (filter = "spoken")}>Говорит</button>
          <button role="radio" aria-checked={filter === "mentioned"} class:on={filter === "mentioned"} onclick={() => (filter = "mentioned")}
            >Упоминания</button
          >
        </div>
      {/if}
    </div>
    <div class="lines">
      {#each groups as group, index (`${group.scene}-${index}`)}
        <!-- Своя обёртка у сцены: её заголовок прилипает, пока видны её реплики, и уступает место следующему. -->
        <div class="group">
        <div class="scene">
          <span>{sceneTitle(group.scene, group.title)}</span>
        </div>
        {#each group.lines as item (item.line.id)}
          <div class="line">
            {#if item.line.location}
              <button class="where" title="Строка {item.line.location} в исходнике" onclick={() => onOpenLine(item, "source")}>{item.line.location}</button>
            {:else}
              <span class="where"></span>
            {/if}
            <button class="text" title="Открыть в таблице перевода" onclick={() => onOpenLine(item, "table")}>
              <span class="speaker" class:own={item.spoken}>{item.line.speaker ?? "Рассказчик"}</span>
              <span class="source">{item.line.sourceText}</span>
              <span class="target" class:placeholder={!item.line.text}>{item.line.text || "Нет перевода"}</span>
            </button>
          </div>
        {/each}
        </div>
      {:else}
        <span class="faint nothing">Таких реплик нет</span>
      {/each}
      {#if shown.length > limit}
        <button class="btn small more" onclick={() => (limit += PAGE)}>Показать ещё {Math.min(PAGE, shown.length - limit)} из {shown.length - limit}</button>
      {/if}
    </div>
  {/if}
</section>

<style>
  /* Область рядом с полями записи: заголовок, поиск и фильтр на месте, листаются только реплики. */
  .entry-lines {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
  }

  .head {
    display: flex;
    gap: 8px;
    align-items: center;
    min-height: 28px;
    padding: 12px 14px 0;
  }

  .head strong {
    color: var(--text-strong);
    font-size: 13px;
    font-weight: 600;
  }

  .summary {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Поиск и фильтр одной строкой: поле сжимается, переключатель нет. */
  .filters {
    display: flex;
    gap: 8px;
    align-items: center;
    padding: 12px 14px 10px;
    border-bottom: 1px solid var(--divider);
  }

  .filters .input {
    flex: 1;
  }

  .filters .segmented {
    flex-shrink: 0;
  }

  .lines {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-height: 0;
    overflow-y: auto;
  }

  .nothing {
    padding: 10px 14px;
  }

  .scene {
    position: sticky;
    top: 0;
    z-index: 1;
    overflow: hidden;
    padding: 8px 14px;
    border-bottom: 1px solid var(--divider-row);
    background: var(--bg);
    color: var(--accent-text);
    font-size: 12px;
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .line {
    display: grid;
    grid-template-columns: 40px minmax(0, 1fr);
    gap: 6px;
    padding: 4px 8px 4px 0;
    border-bottom: 1px solid var(--divider-row);
  }

  .line:hover {
    background: var(--hover-row);
  }

  .where {
    padding: 3px 0 0;
    border: 0;
    background: none;
    color: var(--text-ghost);
    font: inherit;
    font-size: 10px;
    text-align: right;
  }

  button.where {
    align-self: start;
    cursor: pointer;
  }

  button.where:hover {
    color: var(--accent-text);
    text-decoration: underline;
  }

  .text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    padding: 2px 6px;
    border: 1px solid transparent;
    border-radius: 6px;
    background: transparent;
    font-size: 12px;
    line-height: 1.45;
    text-align: left;
    cursor: pointer;
  }

  .text:hover {
    border-color: var(--border);
  }

  .speaker {
    color: var(--text-faint);
    font-size: 11px;
    font-weight: 600;
  }

  .speaker.own {
    color: var(--accent-text);
  }

  .source {
    color: var(--text-soft);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .target {
    color: var(--text);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .target.placeholder {
    color: var(--text-ghost);
    font-style: italic;
  }

  .more {
    align-self: flex-start;
    margin: 8px;
  }
</style>
