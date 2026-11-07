<script lang="ts">
  import { onMount, tick, untrack } from "svelte";
  import { ChevronLeft, ChevronRight, Clapperboard, LoaderCircle, Route, TableProperties, TriangleAlert, X } from "lucide-svelte";
  import Combobox from "./Combobox.svelte";
  import { commands } from "$lib/bindings";
  import {
    NODE_HEIGHT,
    NODE_WIDTH,
    layoutGraph,
    sceneName,
    type Column,
    type GraphEdge,
    type GraphNode,
    type GraphScene,
  } from "$lib/scene-graph";

  let {
    projectId,
    focus,
    onFocus,
    onOpen,
  }: {
    projectId: string;
    /** Ключ сцены в центре. `null`: первая сцена по сюжету. */
    focus: string | null;
    onFocus: (key: string) => void;
    /** Открыть реплики сцены в таблице. */
    onOpen: (scene: GraphScene) => void;
  } = $props();

  const PAGE_SIZES = [10, 25, 50];
  const KINDS: { kind: string; label: string }[] = [
    { kind: "jump", label: "Переход" },
    { kind: "choice", label: "Выбор" },
    { kind: "branch", label: "Условие" },
    { kind: "call", label: "Вызов" },
    { kind: "screen", label: "Экран" },
    { kind: "mixed", label: "Разные виды" },
  ];

  let scenes = $state<GraphScene[]>([]);
  let edges = $state<GraphEdge[]>([]);
  let loading = $state(true);
  let error = $state("");
  let pageSize = $state(10);
  let incomingPage = $state(1);
  let outgoingPage = $state(1);
  let incomingFilter = $state("");
  let outgoingFilter = $state("");
  let pickerText = $state("");
  let zoom = $state(1);
  let viewport = $state<HTMLDivElement>();
  let drag: { x: number; y: number; left: number; top: number } | null = null;
  let dragging = $state(false);

  const graph = $derived(
    layoutGraph(scenes, edges, { focus, pageSize, incomingPage, incomingFilter, outgoingPage, outgoingFilter }),
  );
  /** Отдельно от `graph`: смена страницы колонки пересчитывает граф, но не центр. */
  const center = $derived(graph.center);
  const centerScene = $derived(scenes.find((scene) => scene.key === center));

  onMount(async () => {
    try {
      const data = await commands.sceneGraph(projectId);
      scenes = data.scenes;
      edges = data.edges;
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  });

  // Новая сцена в центре: колонки с первой страницы и без поиска, вид на центр.
  $effect(() => {
    if (!center) return;
    untrack(() => {
      incomingPage = 1;
      outgoingPage = 1;
      incomingFilter = "";
      outgoingFilter = "";
    });
    void tick().then(() => scrollToCenter(false));
  });

  function scrollToCenter(smooth = true) {
    const node = graph.nodes.find((item) => item.role === "center");
    if (!node || !viewport) return;
    viewport.scrollTo({
      left: (node.x + NODE_WIDTH / 2) * zoom - viewport.clientWidth / 2,
      top: (node.y + NODE_HEIGHT / 2) * zoom - viewport.clientHeight / 2,
      behavior: smooth ? "smooth" : "auto",
    });
  }

  async function setZoom(value: number) {
    if (!viewport) return;
    const x = (viewport.scrollLeft + viewport.clientWidth / 2) / zoom;
    const y = (viewport.scrollTop + viewport.clientHeight / 2) / zoom;
    zoom = Math.max(0.3, Math.min(2, value));
    await tick();
    viewport.scrollLeft = x * zoom - viewport.clientWidth / 2;
    viewport.scrollTop = y * zoom - viewport.clientHeight / 2;
  }

  async function fit() {
    if (!viewport) return;
    const scale = Math.min((viewport.clientWidth - 32) / graph.width, (viewport.clientHeight - 32) / graph.height);
    zoom = Math.max(0.3, Math.min(1.2, scale));
    await tick();
    scrollToCenter();
  }

  function startPan(event: PointerEvent) {
    if (!viewport || event.button !== 0 || (event.target as Element).closest("button, input")) return;
    drag = { x: event.clientX, y: event.clientY, left: viewport.scrollLeft, top: viewport.scrollTop };
    dragging = true;
    viewport.setPointerCapture(event.pointerId);
  }

  function pan(event: PointerEvent) {
    if (!drag || !viewport) return;
    viewport.scrollLeft = drag.left - (event.clientX - drag.x);
    viewport.scrollTop = drag.top - (event.clientY - drag.y);
  }

  function endPan() {
    drag = null;
    dragging = false;
  }

  function onKey(event: KeyboardEvent) {
    if (event.target !== viewport || !viewport) return;
    const step: Record<string, [number, number]> = {
      ArrowLeft: [-80, 0],
      ArrowRight: [80, 0],
      ArrowUp: [0, -80],
      ArrowDown: [0, 80],
    };
    const offset = step[event.key];
    if (offset) {
      event.preventDefault();
      viewport.scrollBy(offset[0], offset[1]);
    }
  }

  /** Сцены для выбора центра: по сюжету, поиск по имени и названию. */
  function pickerOptions(all: boolean) {
    const query = pickerText.trim().toLowerCase();
    const found = all || !query ? scenes : scenes.filter((scene) => sceneName(scene.key, scene).toLowerCase().includes(query));
    return found.slice(0, 200).map((scene) => ({ id: scene.key, label: sceneName(scene.key, scene) }));
  }

  const percent = (scene: GraphScene) => (scene.total === 0 ? 0 : Math.round((scene.translated / scene.total) * 100));
</script>

{#snippet columnHead(title: string, info: Column, x: number, filter: string, setFilter: (text: string) => void, setPage: (page: number) => void)}
  <div class="column-head" style:left="{x}px" style:width="{NODE_WIDTH}px">
    <span class="column-title">{title} · {info.total}</span>
    {#if info.total > pageSize || filter}
      <div class="column-tools">
        <input
          class="input column-search"
          value={filter}
          oninput={(event) => setFilter(event.currentTarget.value)}
          placeholder="Найти сцену"
          aria-label={`Поиск: ${title}`}
        />
        {#if info.pages > 1}
          <button class="icon-button" aria-label="Предыдущая страница" title="Предыдущая страница" disabled={info.page <= 1} onclick={() => setPage(info.page - 1)}>
            <ChevronLeft size={14} />
          </button>
          <span class="pages">{info.page}/{info.pages}</span>
          <button class="icon-button" aria-label="Следующая страница" title="Следующая страница" disabled={info.page >= info.pages} onclick={() => setPage(info.page + 1)}>
            <ChevronRight size={14} />
          </button>
        {/if}
      </div>
    {/if}
  </div>
{/snippet}

{#snippet nodeCard(node: GraphNode)}
  {@const scene = node.scene}
  <div
    class="node {node.role}"
    class:outside={!scene}
    style:left="{node.x}px"
    style:top="{node.y}px"
    style:width="{NODE_WIDTH}px"
    style:height="{NODE_HEIGHT}px"
  >
    {#if node.role === "center"}
      <!-- Центр не кнопка: в нём своя кнопка «Реплики», а кнопки не вкладываются друг в друга. -->
      <div class="node-main">
        <span class="node-head">
          {@render nodeName(node, scene)}
          {#if scene}
            <button class="btn small" onclick={() => onOpen(scene)} disabled={!scene.file} title={scene.file ? "Открыть реплики сцены в таблице" : "У сцены нет реплик"}>
              <TableProperties size={13} />
              <span>Реплики</span>
            </button>
          {/if}
        </span>
        {@render nodeBody(scene)}
      </div>
    {:else}
      <button class="node-main" onclick={() => onFocus(node.key)} title={`Перейти к сцене ${sceneName(node.key, scene)}`}>
        <span class="node-head">{@render nodeName(node, scene)}</span>
        {@render nodeBody(scene)}
      </button>
    {/if}
  </div>
{/snippet}

{#snippet nodeName(node: GraphNode, scene: GraphScene | undefined)}
  <span class="node-name">
    {#if scene && scene.total > 0}<Clapperboard size={13} />{:else}<Route size={13} />{/if}
    <span>{sceneName(node.key, scene)}</span>
  </span>
{/snippet}

{#snippet nodeBody(scene: GraphScene | undefined)}
  <span class="node-line">
    {#if !scene}
      Сцены нет в снимке
    {:else if scene.firstLine}
      {scene.firstLine}
    {:else}
      Без реплик: только переходы
    {/if}
  </span>
  {#if scene && scene.total > 0}
    <span class="node-progress">
      <span class="track"><span class="fill" style:width="{percent(scene)}%"></span></span>
      <span class="count">{scene.translated}/{scene.total}{scene.drafts ? ` · модель: ${scene.drafts}` : ""}</span>
    </span>
  {/if}
{/snippet}

<section class="graph">
  <div class="toolbar">
    <div class="picker">
      <Combobox
        text={pickerText}
        options={pickerOptions}
        label="Сцена в центре графа"
        placeholder={center ? sceneName(center, centerScene) : "Найти сцену"}
        empty="Таких сцен нет"
        selected={center ?? ""}
        oninput={(text) => (pickerText = text)}
        onpick={(key) => {
          pickerText = "";
          onFocus(key);
        }}
      />
    </div>
    {#if center}
      <span class="stat">Входов: {graph.incoming.total} · Выходов: {graph.outgoing.total}</span>
    {/if}
    <span class="spacer"></span>
    <div class="segmented" role="group" aria-label="Сцен на странице колонки">
      {#each PAGE_SIZES as size (size)}
        <button class:on={pageSize === size} onclick={() => (pageSize = size)}>{size}</button>
      {/each}
    </div>
    <div class="zoom">
      <button class="icon-button" aria-label="Уменьшить" title="Уменьшить" onclick={() => setZoom(zoom / 1.2)}>−</button>
      <button class="btn small" title="Масштаб 100%" onclick={() => setZoom(1)}>{Math.round(zoom * 100)}%</button>
      <button class="icon-button" aria-label="Увеличить" title="Увеличить" onclick={() => setZoom(zoom * 1.2)}>+</button>
      <button class="btn small" onclick={fit}>По размеру</button>
    </div>
  </div>

  <div class="legend">
    {#each KINDS as item (item.kind)}
      <span class="legend-item"><span class="swatch {item.kind}"></span>{item.label}</span>
    {/each}
    <span class="legend-hint">Щелчок по сцене ставит её в центр · Ctrl и колесо меняют масштаб</span>
  </div>

  {#if error}
    <div class="banner error graph-error" role="alert">
      <TriangleAlert size={15} />
      <span>{error}</span>
      <button class="icon-button" aria-label="Скрыть ошибку" title="Скрыть ошибку" onclick={() => (error = "")}><X size={14} /></button>
    </div>
  {/if}

  {#if loading}
    <div class="empty"><LoaderCircle size={18} class="spin" /> Собираю граф сцен</div>
  {:else if scenes.length === 0}
    <div class="empty">В снимке нет сцен</div>
  {:else}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div
      class="viewport"
      class:dragging
      bind:this={viewport}
      role="application"
      aria-label="Граф сцен"
      tabindex="0"
      onkeydown={onKey}
      onpointerdown={startPan}
      onpointermove={pan}
      onpointerup={endPan}
      onpointercancel={endPan}
      onlostpointercapture={endPan}
      onwheel={(event) => {
        if (event.ctrlKey) {
          event.preventDefault();
          void setZoom(zoom * (event.deltaY > 0 ? 0.9 : 1.1));
        }
      }}
    >
      <div class="extent" style:width="{graph.width * zoom}px" style:height="{graph.height * zoom}px">
        <div class="world" style:width="{graph.width}px" style:height="{graph.height}px" style:transform="scale({zoom})">
          {@render columnHead("Входы", graph.incoming, 40, incomingFilter, (text) => { incomingFilter = text; incomingPage = 1; }, (page) => (incomingPage = page))}
          {@render columnHead("Выходы", graph.outgoing, graph.width - 40 - NODE_WIDTH, outgoingFilter, (text) => { outgoingFilter = text; outgoingPage = 1; }, (page) => (outgoingPage = page))}

          <svg class="links" width={graph.width} height={graph.height} aria-hidden="true">
            {#each graph.links as link (link.id)}
              <path class="link {link.kind}" d={link.path} />
            {/each}
            {#each graph.links as link (link.id)}
              {#if link.label}
                <text class="link-label" x={link.labelX} y={link.labelY} text-anchor={link.labelAnchor}>
                  <title>{link.title}</title>
                  {link.label.length > 30 ? `${link.label.slice(0, 29)}…` : link.label}
                </text>
              {/if}
            {/each}
          </svg>

          {#each graph.nodes as node (`${node.role}:${node.key}`)}
            {@render nodeCard(node)}
          {/each}
        </div>
      </div>
    </div>
  {/if}
</section>

<style>
  .graph {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }

  .toolbar {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 16px;
    border-bottom: 1px solid var(--divider);
  }

  .picker {
    width: 320px;
  }

  .stat,
  .pages {
    color: var(--text-faint);
    font-size: 11px;
    white-space: nowrap;
  }

  .spacer {
    flex: 1;
  }

  .zoom {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .legend {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 12px;
    padding: 6px 16px;
    border-bottom: 1px solid var(--divider-row);
    color: var(--text-faint);
    font-size: 11px;
  }

  .legend-item {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  .legend-hint {
    margin-left: auto;
  }

  .swatch {
    width: 16px;
    height: 0;
    border-top: 2px solid var(--text-faint);
  }

  .swatch.choice {
    border-color: var(--accent);
  }

  .swatch.branch {
    border-color: var(--warning);
  }

  .swatch.call {
    border-color: var(--success);
  }

  .swatch.mixed {
    border-color: var(--text-muted);
  }

  .swatch.screen {
    border-top-style: dashed;
    border-color: var(--accent-text);
  }

  .graph-error {
    margin: 10px 16px 0;
  }

  .empty {
    display: flex;
    flex: 1;
    align-items: center;
    justify-content: center;
    gap: 8px;
    color: var(--text-faint);
    font-size: 12px;
  }

  .viewport {
    flex: 1;
    overflow: auto;
    outline: none;
    cursor: grab;
  }

  .viewport.dragging {
    cursor: grabbing;
    user-select: none;
  }

  .extent {
    position: relative;
  }

  .world {
    position: absolute;
    top: 0;
    left: 0;
    transform-origin: 0 0;
  }

  .links {
    position: absolute;
    inset: 0;
    overflow: visible;
    pointer-events: none;
  }

  .link {
    fill: none;
    stroke: var(--text-faint);
    stroke-width: 1.5;
  }

  .link.choice {
    stroke: var(--accent);
  }

  .link.branch {
    stroke: var(--warning);
  }

  .link.call {
    stroke: var(--success);
  }

  .link.screen {
    stroke: var(--accent-text);
    stroke-dasharray: 5 4;
  }

  .link.mixed {
    stroke: var(--text-muted);
  }

  .link-label {
    fill: var(--text-soft);
    font-size: 11px;
    paint-order: stroke;
    stroke: var(--bg);
    stroke-width: 4px;
    pointer-events: auto;
    cursor: help;
  }

  .column-head {
    position: absolute;
    top: 16px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .column-title {
    color: var(--text-muted);
    font-size: 11px;
    font-weight: 600;
  }

  .column-tools {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .column-search {
    flex: 1;
    padding: 5px 8px;
    font-size: 11px;
  }

  .node {
    position: absolute;
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border);
    border-radius: 10px;
    background: var(--surface);
  }

  .node:hover {
    border-color: var(--border-selected);
  }

  .node.center {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent-ring);
  }

  .node.outside {
    border-style: dashed;
    background: var(--bg);
  }

  .node-main {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 6px;
    min-height: 0;
    padding: 10px 12px;
    border: 0;
    border-radius: 10px;
    background: transparent;
    color: inherit;
    text-align: left;
  }

  .node-name {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    color: var(--text);
    font-size: 12px;
    font-weight: 600;
  }

  .node-name span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .node-line {
    display: -webkit-box;
    overflow: hidden;
    color: var(--text-faint);
    font-size: 11px;
    line-height: 1.45;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
  }

  .node-progress {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: auto;
  }

  .track {
    flex: 1;
    height: 4px;
    overflow: hidden;
    border-radius: 999px;
    background: var(--track);
  }

  .fill {
    display: block;
    height: 100%;
    background: var(--accent);
  }

  .count {
    color: var(--text-faint);
    font-size: 10px;
    white-space: nowrap;
  }

  /* Название и кнопка «Реплики» в одну строку: кнопка не ложится на текст реплики. */
  .node-head {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }

  .node-head .node-name {
    flex: 1;
  }
</style>
