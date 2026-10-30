<script lang="ts">
  import { tick } from 'svelte';
  import {
    buildSceneGraph,
    NODE_WIDTH,
    NODE_HEIGHT,
    GAP_X,
    type GraphScene,
    type GraphEdge,
    type GraphNode,
  } from './scene-graph';

  let {
    scenes,
    edges,
    selectedId,
    onselect,
    onnavigate,
    onopen,
    busy = false,
  }: {
    scenes: GraphScene[];
    edges: GraphEdge[];
    selectedId: string;
    onselect: (scene: GraphScene) => void;
    onnavigate: (name: string) => void;
    onopen: (id: string) => void;
    busy?: boolean;
  } = $props();

  let graphMode = $state<'focused' | 'all'>('focused');
  let zoom = $state(1);
  let viewport: HTMLDivElement;
  let drag: { x: number; y: number; left: number; top: number } | null = null;
  let dragging = $state(false);

  // Выбранная сцена (если "all" или не найдена — берем первую)
  let selected = $derived(
    (selectedId && selectedId !== 'all' ? scenes.find(s => s.id === selectedId) : null) ||
    scenes[0]
  );

  let graph = $derived(
    buildSceneGraph(scenes, edges, {
      mode: graphMode,
      focusName: selected?.name ?? null,
    })
  );

  let activeLinks = $derived(
    graph.links.filter(
      edge => edge.from === selected?.name || edge.to === selected?.name
    )
  );

  function centerOnNode(name: string) {
    const node = graph.nodes.find(n => n.name === name);
    if (!node || !viewport) return;
    viewport.scrollTo({
      left: (node.x + NODE_WIDTH / 2) * zoom - viewport.clientWidth / 2,
      top: (node.y + NODE_HEIGHT / 2) * zoom - viewport.clientHeight / 2,
      behavior: 'smooth',
    });
  }

  $effect(() => {
    const name = selected?.name;
    if (name) {
      void tick().then(() => {
        if (graphMode === 'focused') {
          // В сфокусированном режиме центрируем на центральном узле
          const centerNode = graph.nodes.find(n => n.role === 'center');
          if (centerNode && viewport) {
            viewport.scrollTo({
              left: (centerNode.x + NODE_WIDTH / 2) * zoom - viewport.clientWidth / 2,
              top: (centerNode.y + NODE_HEIGHT / 2) * zoom - viewport.clientHeight / 2,
              behavior: 'smooth',
            });
          }
        } else {
          centerOnNode(name);
        }
      });
    }
  });

  async function setZoom(value: number) {
    const old = zoom;
    const x = (viewport.scrollLeft + viewport.clientWidth / 2) / old;
    const y = (viewport.scrollTop + viewport.clientHeight / 2) / old;
    zoom = Math.max(0.2, Math.min(2, value));
    await tick();
    viewport.scrollLeft = x * zoom - viewport.clientWidth / 2;
    viewport.scrollTop = y * zoom - viewport.clientHeight / 2;
  }

  async function fit() {
    if (!viewport) return;
    const scaleX = (viewport.clientWidth - 40) / graph.width;
    const scaleY = (viewport.clientHeight - 40) / graph.height;
    zoom = Math.max(0.25, Math.min(1.2, Math.min(scaleX, scaleY)));
    await tick();
    viewport.scrollTo({
      left: Math.max(0, (graph.width * zoom - viewport.clientWidth) / 2),
      top: Math.max(0, (graph.height * zoom - viewport.clientHeight) / 2),
      behavior: 'smooth',
    });
  }

  function startPan(event: PointerEvent) {
    if (event.button !== 0 || (event.target as Element).closest('button')) return;
    drag = {
      x: event.clientX,
      y: event.clientY,
      left: viewport.scrollLeft,
      top: viewport.scrollTop,
    };
    dragging = true;
    viewport.setPointerCapture(event.pointerId);
  }

  function pan(event: PointerEvent) {
    if (!drag) return;
    viewport.scrollLeft = drag.left - (event.clientX - drag.x);
    viewport.scrollTop = drag.top - (event.clientY - drag.y);
  }

  function endPan() {
    drag = null;
    dragging = false;
  }

  function handleNodeClick(node: GraphNode) {
    if (node.scene) {
      onselect(node.scene);
    } else {
      onnavigate(node.name);
    }
  }
</script>

<div class="graph-view">
  <!-- Верхний тулбар графа -->
  <div class="toolbar">
    <div class="mode-toggles">
      <button
        class="mode-btn"
        class:btn-active={graphMode === 'focused'}
        onclick={() => graphMode = 'focused'}
        title="Локальный граф: только текущая сцена и связанные с ней входы/выходы (быстро и без лагов)"
      >
        🎯 Только связанные
      </button>
      <button
        class="mode-btn"
        class:btn-active={graphMode === 'all'}
        onclick={() => graphMode = 'all'}
        title="Показать все сцены файла целиком"
      >
        🌐 Весь граф
      </button>
    </div>

    <div class="toolbar-info">
      {#if graphMode === 'focused'}
        {#if selected}
          <span>
            В фокусе: <strong class="text-sky-400">{selected.name}</strong>
            <span class="info-sep">·</span>
            ← Входящих: <strong>{graph.incomingCount}</strong>
            <span class="info-sep">·</span>
            Исходящих: <strong>{graph.outgoingCount}</strong> →
          </span>
        {/if}
      {:else}
        <span>{graph.nodes.length} узлов · {graph.links.length} связей</span>
      {/if}
    </div>

    <div class="zoom-controls">
      <button onclick={() => setZoom(zoom / 1.2)} aria-label="Уменьшить масштаб">−</button>
      <button onclick={() => setZoom(1)} title="Масштаб 100%">{Math.round(zoom * 100)}%</button>
      <button onclick={() => setZoom(zoom * 1.2)} aria-label="Увеличить масштаб">+</button>
      <button onclick={fit}>По размеру</button>
      {#if selected}
        <button onclick={() => centerOnNode(selected.name)}>К фокусу</button>
      {/if}
    </div>
  </div>

  <div class="hint">
    {#if graphMode === 'focused'}
      💡 Кликните на левый или правый узел, чтобы моментально переключить фокус на него · Колесико с Ctrl — масштаб
    {:else}
      💡 Перетаскивайте фон · Ctrl + колесо — масштаб · Выберите узел для просмотра связей
    {/if}
  </div>

  <!-- Область холста (Viewport) -->
  <div
    class="viewport"
    class:dragging
    bind:this={viewport}
    role="application"
    aria-label="Граф сценария"
    tabindex="0"
    onkeydown={(event) => {
      if (event.target !== viewport) return;
      const offsets: Record<string, [number, number]> = {
        ArrowLeft: [-80, 0],
        ArrowRight: [80, 0],
        ArrowUp: [0, -80],
        ArrowDown: [0, 80],
      };
      const offset = offsets[event.key];
      if (offset) {
        event.preventDefault();
        viewport.scrollBy(offset[0], offset[1]);
      }
    }}
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
    {#if graph.nodes.length === 0}
      <p class="empty">В этом файле нет сцен для построения графа.</p>
    {/if}

    <div class="extent" style:width={`${graph.width * zoom}px`} style:height={`${graph.height * zoom}px`}>
      <div
        class="world"
        style:width={`${graph.width}px`}
        style:height={`${graph.height}px`}
        style:transform={`scale(${zoom})`}
      >
        <!-- Заголовки колонок в сфокусированном режиме -->
        {#if graphMode === 'focused'}
          <div class="column-header-banner banner-in" style:left="60px" style:width="{NODE_WIDTH}px">
            <span class="banner-badge">← Входящие переходы ({graph.incomingCount})</span>
          </div>
          <div class="column-header-banner banner-center" style:left="{60 + NODE_WIDTH + GAP_X}px" style:width="{NODE_WIDTH}px">
            <span class="banner-badge badge-active">★ Текущая сцена в фокусе</span>
          </div>
          <div class="column-header-banner banner-out" style:left="{60 + (NODE_WIDTH + GAP_X) * 2}px" style:width="{NODE_WIDTH}px">
            <span class="banner-badge">Исходящие развилки ({graph.outgoingCount}) →</span>
          </div>

          <!-- Пустые состояния колонок при отсутствии входящих или исходящих -->
          {#if graph.incomingCount === 0}
            <div class="empty-state-column" style:left="60px" style:top="{graph.height / 2 - 45}px" style:width="{NODE_WIDTH}px">
              <span class="empty-emoji">🏁</span>
              <strong>Начальная сцена</strong>
              <small>Нет входящих переходов в текущем файле</small>
            </div>
          {/if}

          {#if graph.outgoingCount === 0}
            <div class="empty-state-column" style:left="{60 + (NODE_WIDTH + GAP_X) * 2}px" style:top="{graph.height / 2 - 45}px" style:width="{NODE_WIDTH}px">
              <span class="empty-emoji">🏁</span>
              <strong>Конечная точка</strong>
              <small>Нет исходящих переходов (конец скрипта/рута)</small>
            </div>
          {/if}
        {/if}

        <!-- SVG-связи между узлами -->
        <svg width={graph.width} height={graph.height} aria-label="Связи между сценами">
          <defs>
            <marker
              id="scene-arrow"
              markerWidth="8"
              markerHeight="8"
              refX="7"
              refY="4"
              orient="auto-start-reverse"
            >
              <path d="M 0 0 L 8 4 L 0 8 z" fill="#94a3b8" />
            </marker>
            <marker
              id="scene-arrow-active"
              markerWidth="8"
              markerHeight="8"
              refX="7"
              refY="4"
              orient="auto-start-reverse"
            >
              <path d="M 0 0 L 8 4 L 0 8 z" fill="#38bdf8" />
            </marker>
          </defs>

          {#each graph.links as edge (edge.id)}
            {@const isRelated = edge.from === selected?.name || edge.to === selected?.name}
            <g
              class:related={isRelated}
              class:branch={edge.kind === 'choice' || edge.kind === 'branch'}
            >
              <path
                d={edge.path}
                fill="none"
                stroke-width={isRelated ? "2.5" : "1.8"}
                marker-end={isRelated ? "url(#scene-arrow-active)" : "url(#scene-arrow)"}
              />
              <text x={edge.labelX} y={edge.labelY} text-anchor="middle">
                <title>{edge.condition || edge.kind}: {edge.from} → {edge.to}</title>
                {(edge.condition || edge.kind).slice(0, 28)}{(edge.condition?.length ?? 0) > 28 ? '…' : ''}
              </text>
            </g>
          {/each}
        </svg>

        <!-- Узлы графа -->
        {#each graph.nodes as node (node.id)}
          {@const isCenter = node.role === 'center' || node.scene?.id === selected?.id}
          {@const isExternal = !node.scene}
          {@const pct = node.scene && node.scene.total_lines > 0
            ? Math.round((node.scene.translated_lines / node.scene.total_lines) * 100)
            : 0}
          <div
            id={node.scene ? `scene-node-${node.scene.id}` : `scene-node-${node.name}`}
            class="node"
            class:role-center={isCenter}
            class:role-incoming={node.role === 'incoming'}
            class:role-outgoing={node.role === 'outgoing'}
            class:external={isExternal}
            style:left={`${node.x}px`}
            style:top={`${node.y}px`}
            style:width={`${NODE_WIDTH}px`}
            style:height={`${NODE_HEIGHT}px`}
          >
            <button
              class="node-main"
              disabled={busy && isExternal}
              onclick={() => handleNodeClick(node)}
              title={node.name}
            >
              <div class="node-badge-row">
                {#if isCenter}
                  <span class="badge-role role-badge-center">★ В фокусе</span>
                {:else if node.role === 'incoming'}
                  <span class="badge-role" class:role-badge-in={!isExternal} class:role-badge-in-ext={isExternal}>
                    {isExternal ? '← Внешний вход' : '← Вход'}
                  </span>
                {:else if node.role === 'outgoing'}
                  <span class="badge-role role-badge-out">
                    {isExternal ? 'Внешний переход →' : 'Переход →'}
                  </span>
                {/if}
                <small class="node-type-label">
                  {node.scene
                    ? (node.scene.choices?.length ? '◆ Развилка' : 'Сцена')
                    : '↗ Другой файл'}
                </small>
              </div>

              <strong class="node-name">{node.name}</strong>

              <span class="preview">
                {node.scene?.entries[0]?.original_text ||
                  (isExternal ? 'Метка в другом файле: нажмите для перехода' : 'Нет диалоговых реплик')}
              </span>
            </button>

            {#if node.scene}
              <div class="node-footer">
                <span class="node-stats">
                  {node.scene.translated_lines}/{node.scene.total_lines} строк ({pct}%)
                </span>
                <button
                  class="btn-open-dialogues"
                  onclick={() => onopen(node.scene!.id)}
                  title="Открыть диалоги сцены в таблице"
                >
                  Диалоги →
                </button>
              </div>
            {:else}
              <div class="node-footer external-footer">
                <span>{busy ? 'Поиск...' : 'Другой файл проекта'}</span>
              </div>
            {/if}
          </div>
        {/each}
      </div>
    </div>
  </div>

  <!-- Нижняя панель детальных связей текущей сцены -->
  {#if selected}
    <div class="details">
      <div class="details-title">
        <span>Связи сцены: <strong class="text-sky-300">{selected.name}</strong></span>
      </div>

      {#if activeLinks.length === 0}
        <span class="no-links-text">У сцены нет зафиксированных переходов.</span>
      {/if}

      <div class="details-links-row">
        {#each activeLinks as edge (edge.id)}
          {@const outgoing = edge.from === selected.name}
          <button
            class="link-pill"
            class:pill-out={outgoing}
            class:pill-in={!outgoing}
            disabled={busy}
            onclick={() => onnavigate(outgoing ? edge.to : edge.from)}
            title="Перейти к сцене"
          >
            <span class="pill-arrow">{outgoing ? '→' : '←'}</span>
            <span class="pill-target">{outgoing ? edge.to : edge.from}</span>
            <small class="pill-cond">{edge.condition || edge.kind}</small>
          </button>
        {/each}

        {#each selected.choices ?? [] as choice}
          {#each choice.options.filter(option => !option.target_jump) as option}
            <span class="link-pill pill-dead-end" title="У варианта не указан переход jump">
              ◆ {option.text} <small>(локальный блок)</small>
            </span>
          {/each}
        {/each}
      </div>
    </div>
  {/if}
</div>

<style>
  .graph-view {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    background: #090d16;
    color: #cbd5e1;
    user-select: none;
  }

  .toolbar {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 14px;
    background: #0f172a;
    border-bottom: 1px solid #1e293b;
    flex-wrap: wrap;
  }

  .mode-toggles {
    display: inline-flex;
    background: #090d16;
    padding: 3px;
    border-radius: 7px;
    border: 1px solid #1e293b;
    gap: 3px;
  }

  .mode-btn {
    border: none;
    background: transparent;
    color: #94a3b8;
    padding: 4px 10px;
    border-radius: 5px;
    font-size: 11.5px;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .mode-btn:hover {
    color: #f1f5f9;
  }

  .mode-btn.btn-active {
    background: #1e293b;
    color: #38bdf8;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.4);
  }

  .toolbar-info {
    margin-right: auto;
    font-size: 12px;
    color: #94a3b8;
  }

  .toolbar-info strong {
    color: #e2e8f0;
  }

  .info-sep {
    color: #475569;
    margin: 0 4px;
  }

  .zoom-controls {
    display: flex;
    align-items: center;
    gap: 5px;
  }

  button {
    color: #cbd5e1;
    background: #131d2e;
    border: 1px solid #293548;
    border-radius: 5px;
    padding: 4px 9px;
    cursor: pointer;
    font: inherit;
    font-size: 11.5px;
  }

  button:hover {
    border-color: #38bdf8;
    color: #fff;
  }

  button:focus-visible {
    outline: 2px solid #38bdf8;
    outline-offset: 2px;
  }

  button:disabled {
    opacity: 0.5;
    cursor: wait;
  }

  .hint {
    padding: 5px 14px;
    color: #64748b;
    font-size: 11px;
    background: #0b111e;
    border-bottom: 1px solid #162032;
  }

  .viewport {
    flex: 1;
    min-height: 200px;
    overflow: auto;
    cursor: grab;
    touch-action: none;
    background-image: radial-gradient(#1e293b 1px, transparent 1px);
    background-size: 24px 24px;
    position: relative;
  }

  .viewport.dragging {
    cursor: grabbing;
  }

  .extent, .world {
    position: relative;
  }

  .world {
    transform-origin: 0 0;
  }

  /* Заголовки колонок в focused режиме */
  .column-header-banner {
    position: absolute;
    top: 25px;
    text-align: center;
    pointer-events: none;
    z-index: 5;
  }

  .banner-badge {
    display: inline-block;
    padding: 4px 12px;
    border-radius: 20px;
    background: #131d2d;
    border: 1px solid #293548;
    color: #94a3b8;
    font-size: 11px;
    font-weight: 600;
  }

  .banner-badge.badge-active {
    background: #0c2742;
    border-color: #0284c7;
    color: #38bdf8;
  }

  /* Пустые колонки */
  .empty-state-column {
    position: absolute;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 16px;
    border: 1px dashed #293548;
    border-radius: 10px;
    background: #0f172a50;
    text-align: center;
    color: #64748b;
    pointer-events: none;
  }

  .empty-emoji {
    font-size: 20px;
    margin-bottom: 4px;
  }

  .empty-state-column strong {
    font-size: 12px;
    color: #94a3b8;
  }

  .empty-state-column small {
    font-size: 10.5px;
    margin-top: 2px;
  }

  /* SVG Links */
  svg {
    position: absolute;
    inset: 0;
    overflow: visible;
    pointer-events: none;
  }

  g {
    stroke: #475569;
  }

  g.branch {
    stroke: #8b5cf6;
  }

  g.related {
    stroke: #0284c7;
  }

  text {
    fill: #e2e8f0;
    stroke: #090d16;
    stroke-width: 4px;
    paint-order: stroke;
    font: 11px system-ui, sans-serif;
    font-weight: 500;
  }

  /* Карточки узлов */
  .node {
    position: absolute;
    box-sizing: border-box;
    border: 1px solid #293548;
    border-radius: 10px;
    background: #0f172a;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.4);
    transition: border-color 0.15s, box-shadow 0.15s, transform 0.1s;
  }

  .node:hover {
    border-color: #475569;
    transform: translateY(-1px);
  }

  .node.role-center {
    border: 2px solid #38bdf8;
    background: #0c1a2e;
    box-shadow: 0 0 24px rgba(56, 189, 248, 0.25);
    z-index: 10;
  }

  .node.role-incoming {
    border-color: #1e3a5f;
  }

  .node.role-incoming.external {
    border: 1px dashed #38bdf8;
    background: #091a2e;
  }

  .node.role-outgoing {
    border-color: #2a2c54;
  }

  .node.role-outgoing.external {
    border: 1px dashed #d97706;
    background: #1c150c;
  }

  .node.external {
    border: 1px dashed #d97706;
    background: #1c150c;
  }

  .node-main {
    display: flex;
    flex-direction: column;
    gap: 6px;
    text-align: left;
    border: none;
    border-radius: 0;
    background: transparent;
    padding: 10px 12px;
    flex: 1;
    min-height: 0;
    cursor: pointer;
  }

  .node-badge-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .badge-role {
    font-size: 9.5px;
    padding: 2px 6px;
    border-radius: 4px;
    font-weight: 700;
    text-transform: uppercase;
  }

  .role-badge-center {
    background: #0284c7;
    color: #fff;
  }

  .role-badge-in {
    background: #1e293b;
    color: #94a3b8;
    border: 1px solid #334155;
  }

  .role-badge-in-ext {
    background: #0c2f4d;
    color: #38bdf8;
    border: 1px solid #0284c7;
  }

  .role-badge-out {
    background: #2e1065;
    color: #c084fc;
    border: 1px solid #581c87;
  }

  .node-type-label {
    color: #64748b;
    font-size: 10px;
    margin-left: auto;
  }

  .node-name {
    width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: #f1f5f9;
    font-size: 13px;
  }

  .preview {
    display: -webkit-box;
    line-clamp: 2;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    color: #94a3b8;
    font-size: 11px;
    line-height: 1.35;
  }

  .node-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    border-top: 1px solid #1e293b;
    padding: 5px 10px;
    font-size: 11px;
    background: #0a0f1d;
  }

  .node-stats {
    color: #64748b;
    font-size: 10.5px;
  }

  .btn-open-dialogues {
    padding: 2px 7px;
    font-size: 10.5px;
    background: #1e293b;
    border-color: #334155;
  }

  .btn-open-dialogues:hover {
    background: #0284c7;
    border-color: #0284c7;
    color: #fff;
  }

  .external-footer {
    color: #d97706;
    font-size: 10.5px;
  }

  /* Нижняя панель деталей */
  .details {
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-height: 140px;
    overflow-y: auto;
    padding: 10px 14px;
    background: #090d16;
    border-top: 1px solid #1e293b;
    font-size: 12px;
  }

  .details-title {
    font-size: 12px;
    color: #94a3b8;
  }

  .details-links-row {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }

  .link-pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 3px 9px;
    border-radius: 6px;
    font-size: 11.5px;
    background: #131d2e;
    border: 1px solid #293548;
    cursor: pointer;
  }

  .link-pill:hover {
    border-color: #38bdf8;
  }

  .pill-arrow {
    font-weight: 700;
  }

  .pill-in .pill-arrow {
    color: #38bdf8;
  }

  .pill-out .pill-arrow {
    color: #a855f7;
  }

  .pill-target {
    color: #e2e8f0;
    font-weight: 500;
  }

  .pill-cond {
    color: #94a3b8;
    font-size: 10px;
  }

  .pill-dead-end {
    color: #64748b;
    background: #0f172a;
    border: 1px dashed #334155;
    cursor: default;
  }

  .no-links-text {
    color: #64748b;
    font-size: 11.5px;
  }

  .empty {
    padding: 24px;
    color: #64748b;
    text-align: center;
  }
</style>
