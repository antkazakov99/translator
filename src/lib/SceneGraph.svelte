<script lang="ts">
  import { tick } from 'svelte';
  import { FileCode, Loader2, Clapperboard } from 'lucide-svelte';
  import SceneNodeIcon from './SceneNodeIcon.svelte';
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
    onviewcode,
    onback,
    showSidebar = false,
    ontogglesidebar = null,
    busy = false,
    busyTarget = null,
  }: {
    scenes: GraphScene[];
    edges: GraphEdge[];
    selectedId: string;
    onselect: (scene: GraphScene) => void;
    onnavigate: (name: string) => void;
    onopen: (id: string) => void;
    onviewcode?: (scene: GraphScene) => void;
    onback?: (() => void) | null;
    showSidebar?: boolean;
    ontogglesidebar?: (() => void) | null;
    busy?: boolean;
    busyTarget?: string | null;
  } = $props();

  let localBusyTarget = $state<string | null>(null);

  // Сбрасываем локальный таргет поиска, когда операция навигации завершена
  $effect(() => {
    if (!busy) {
      localBusyTarget = null;
    }
  });

  let zoom = $state(1);
  let viewport: HTMLDivElement;
  let drag: { x: number; y: number; left: number; top: number } | null = null;
  let dragging = $state(false);

  // Пагинация и фильтрация для входящих и исходящих узлов (предотвращает лаги на хаб-нодах вроде house)
  let incomingPage = $state(1);
  let incomingFilter = $state('');
  let outgoingPage = $state(1);
  let outgoingFilter = $state('');
  let pageSize = $state(10);

  // Выбранная сцена (если "all" или не найдена — берем первую)
  let selected = $derived(
    (selectedId && selectedId !== 'all' ? scenes.find(s => s.id === selectedId) : null) ||
    scenes[0]
  );

  // Сбрасываем пагинацию и фильтр при переключении сцены
  $effect(() => {
    const _ = selected?.name;
    incomingPage = 1;
    incomingFilter = '';
    outgoingPage = 1;
    outgoingFilter = '';
  });

  let graph = $derived(
    buildSceneGraph(scenes, edges, {
      focusName: selected?.name ?? null,
      incomingPage,
      incomingPageSize: pageSize,
      incomingFilter,
      outgoingPage,
      outgoingPageSize: pageSize,
      outgoingFilter,
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
        const centerNode = graph.nodes.find(n => n.role === 'center');
        if (centerNode && viewport) {
          viewport.scrollTo({
            left: (centerNode.x + NODE_WIDTH / 2) * zoom - viewport.clientWidth / 2,
            top: (centerNode.y + NODE_HEIGHT / 2) * zoom - viewport.clientHeight / 2,
            behavior: 'smooth',
          });
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
      localBusyTarget = node.name;
      onnavigate(node.name);
    }
  }
</script>

<div class="graph-view">
  <!-- Верхний тулбар графа -->
  <div class="toolbar">
    <div class="toolbar-info">
      {#if !showSidebar && ontogglesidebar}
        <button
          type="button"
          class="btn-sidebar-burger"
          onclick={ontogglesidebar}
          title="Показать список сцен"
        >
          <Clapperboard size={14} />
        </button>
        <div class="subtoolbar-divider"></div>
      {/if}
      {#if onback}
        <button
          type="button"
          class="btn-graph-back"
          onclick={onback}
          disabled={busy}
          title="Вернуться к предыдущему графу сценария"
        >
          ← Назад к графу
        </button>
        <div class="subtoolbar-divider"></div>
      {/if}
      {#if selected}
        <div class="toolbar-scene-heading">
          <SceneNodeIcon scene={selected} size={13} />
          <span class="focus-title-badge">Сцена:</span>
          <strong class="focus-scene-name" title={selected.name}>{selected.name}</strong>
          {#if onviewcode}
            <button
              type="button"
              class="btn-toolbar-code"
              onclick={() => onviewcode?.(selected!)}
              title="Посмотреть исходный код сцены в файле (.rpy)"
            >
              <FileCode size={11} />
              <span>Код</span>
            </button>
          {/if}
        </div>
        <div class="subtoolbar-divider"></div>
        <span class="info-stat">← Входов: <strong>{graph.incomingTotalRaw}</strong></span>
        <div class="subtoolbar-divider"></div>
        <span class="info-stat">Выходов: <strong>{graph.outgoingTotalRaw}</strong> →</span>
      {/if}
    </div>

    <div class="zoom-controls">
      <div class="page-size-picker" title="Лимит отображения узлов на страницу">
        <span class="page-size-label">По:</span>
        {#each [10, 25, 50] as size}
          <button
            class="page-size-btn"
            class:size-active={pageSize === size}
            onclick={() => { pageSize = size; incomingPage = 1; outgoingPage = 1; }}
          >{size}</button>
        {/each}
      </div>
      <div class="subtoolbar-divider"></div>
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
    💡 Кликните на входящий (слева) или исходящий (справа) узел для перехода к нему · Колесико с Ctrl — масштаб
  </div>

  <!-- Область холста (Viewport) -->
  <!-- Viewport принимает фокус для прокрутки стрелками и жесты указателя для панорамирования. -->
  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
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
        <!-- Левая колонка: Входящие переходы -->
          <div class="column-header-banner banner-in" style:left="60px" style:width="{NODE_WIDTH}px">
            <div class="banner-top-row">
              <span class="banner-badge">
                ← Входящие ({graph.incomingTotalRaw})
              </span>
            </div>

            {#if graph.incomingTotalRaw > 5 || incomingFilter}
              <div class="column-toolbar">
                <div class="col-search-box">
                  <input
                    type="text"
                    class="col-search-input"
                    placeholder="Поиск входа..."
                    bind:value={incomingFilter}
                    oninput={() => incomingPage = 1}
                  />
                  {#if incomingFilter}
                    <button
                      type="button"
                      class="col-clear-btn"
                      onclick={() => { incomingFilter = ''; incomingPage = 1; }}
                      title="Очистить"
                    >✕</button>
                  {/if}
                </div>

                {#if graph.incomingTotalPages > 1}
                  <div class="col-pager">
                    <button
                      type="button"
                      class="col-page-btn"
                      disabled={incomingPage <= 1}
                      onclick={() => incomingPage--}
                      title="Предыдущая страница"
                    >◀</button>
                    <span class="col-page-num">{incomingPage}/{graph.incomingTotalPages}</span>
                    <button
                      type="button"
                      class="col-page-btn"
                      disabled={incomingPage >= graph.incomingTotalPages}
                      onclick={() => incomingPage++}
                      title="Следующая страница"
                    >▶</button>
                  </div>
                {/if}
              </div>
            {/if}
          </div>

          <!-- Центральная колонка: Фокус -->
          <div class="column-header-banner banner-center" style:left="{60 + NODE_WIDTH + GAP_X}px" style:width="{NODE_WIDTH}px">
            <div class="banner-top-row">
              <span class="banner-badge badge-active">★ Текущая сцена в фокусе</span>
            </div>
          </div>

          <!-- Правая колонка: Исходящие переходы -->
          <div class="column-header-banner banner-out" style:left="{60 + (NODE_WIDTH + GAP_X) * 2}px" style:width="{NODE_WIDTH}px">
            <div class="banner-top-row">
              <span class="banner-badge">
                Исходящие ({graph.outgoingTotalRaw}) →
              </span>
            </div>

            {#if graph.outgoingTotalRaw > 5 || outgoingFilter}
              <div class="column-toolbar">
                <div class="col-search-box">
                  <input
                    type="text"
                    class="col-search-input"
                    placeholder="Поиск выхода..."
                    bind:value={outgoingFilter}
                    oninput={() => outgoingPage = 1}
                  />
                  {#if outgoingFilter}
                    <button
                      type="button"
                      class="col-clear-btn"
                      onclick={() => { outgoingFilter = ''; outgoingPage = 1; }}
                      title="Очистить"
                    >✕</button>
                  {/if}
                </div>

                {#if graph.outgoingTotalPages > 1}
                  <div class="col-pager">
                    <button
                      type="button"
                      class="col-page-btn"
                      disabled={outgoingPage <= 1}
                      onclick={() => outgoingPage--}
                      title="Предыдущая страница"
                    >◀</button>
                    <span class="col-page-num">{outgoingPage}/{graph.outgoingTotalPages}</span>
                    <button
                      type="button"
                      class="col-page-btn"
                      disabled={outgoingPage >= graph.outgoingTotalPages}
                      onclick={() => outgoingPage++}
                      title="Следующая страница"
                    >▶</button>
                  </div>
                {/if}
              </div>
            {/if}
          </div>

          <!-- Пустые состояния колонок при отсутствии входящих или исходящих -->
          {#if graph.incomingCount === 0}
            <div class="empty-state-column" style:left="60px" style:top="{graph.height / 2 - 45}px" style:width="{NODE_WIDTH}px">
              <span class="empty-emoji">{incomingFilter ? '🔍' : '🏁'}</span>
              <strong>{incomingFilter ? 'Ничего не найдено' : 'Начальная сцена'}</strong>
              <small>{incomingFilter ? `Нет входов по фильтру "${incomingFilter}"` : 'Нет входящих переходов в текущем файле'}</small>
              {#if incomingFilter}
                <button type="button" class="col-reset-btn" onclick={() => { incomingFilter = ''; incomingPage = 1; }}>Сбросить фильтр</button>
              {/if}
            </div>
          {/if}

          {#if graph.outgoingCount === 0}
            <div class="empty-state-column" style:left="{60 + (NODE_WIDTH + GAP_X) * 2}px" style:top="{graph.height / 2 - 45}px" style:width="{NODE_WIDTH}px">
              <span class="empty-emoji">{outgoingFilter ? '🔍' : '🏁'}</span>
              <strong>{outgoingFilter ? 'Ничего не найдено' : 'Конечная точка'}</strong>
              <small>{outgoingFilter ? `Нет выходов по фильтру "${outgoingFilter}"` : 'Нет исходящих переходов (конец скрипта/рута)'}</small>
              {#if outgoingFilter}
                <button type="button" class="col-reset-btn" onclick={() => { outgoingFilter = ''; outgoingPage = 1; }}>Сбросить фильтр</button>
              {/if}
            </div>
          {/if}

        <!-- SVG-связи между узлами (под карточками) -->
        <svg class="graph-connections-svg" width={graph.width} height={graph.height} aria-hidden="true">
          <defs>
            <marker
              id="scene-arrow"
              markerWidth="8"
              markerHeight="8"
              refX="7"
              refY="4"
              orient="auto-start-reverse"
            >
              <path d="M 0 0 L 8 4 L 0 8 z" fill="#64748b" />
            </marker>
            <marker
              id="scene-arrow-choice"
              markerWidth="8"
              markerHeight="8"
              refX="7"
              refY="4"
              orient="auto-start-reverse"
            >
              <path d="M 0 0 L 8 4 L 0 8 z" fill="#8b5cf6" />
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
            {@const isChoice = edge.kind === 'choice' || edge.kind === 'branch'}
            <g
              class="edge-path-group"
              class:related={isRelated}
              class:branch={isChoice}
            >
              <path
                d={edge.path}
                fill="none"
                stroke-width={isRelated ? "2.5" : "1.8"}
                marker-end={isRelated ? "url(#scene-arrow-active)" : isChoice ? "url(#scene-arrow-choice)" : "url(#scene-arrow)"}
              />
            </g>
          {/each}
        </svg>

        <!-- Узлы графа -->
        {#each graph.nodes as node (node.id)}
          {@const isCenter = node.role === 'center' || node.scene?.id === selected?.id}
          {@const isExternal = !node.scene}
          {@const isThisNodeBusy = isExternal && busy && ((busyTarget != null && busyTarget === node.name) || localBusyTarget === node.name)}
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
            class:navigating={isThisNodeBusy}
            style:left={`${node.x}px`}
            style:top={`${node.y}px`}
            style:width={`${NODE_WIDTH}px`}
            style:height={`${NODE_HEIGHT}px`}
          >
            <button
              class="node-main"
              class:is-navigating={isThisNodeBusy}
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

              <strong class="node-name">
                <SceneNodeIcon scene={node.scene} isExternal={isExternal} size={13} />
                <span>{node.name}</span>
              </strong>

              <span class="preview">
                {node.scene?.entries[0]?.original_text ||
                  (isExternal
                    ? (isThisNodeBusy ? 'Поиск перехода по файлам проекта...' : 'Метка в другом файле: нажмите для перехода')
                    : (node.scene?.choices?.length ? 'Меню выбора (без реплик персонажей)' : 'Маршрутизатор сюжета (без реплик)'))}
              </span>
            </button>

            {#if node.scene}
              <div class="node-footer">
                {#if node.scene.total_lines > 0}
                  <span class="node-stats">
                    {node.scene.translated_lines}/{node.scene.total_lines} строк ({pct}%)
                  </span>
                  <div class="node-footer-btns">
                    {#if onviewcode}
                      <button
                        type="button"
                        class="btn-code-node"
                        onclick={(e) => { e.stopPropagation(); onviewcode?.(node.scene!); }}
                        title="Посмотреть исходный код сцены в файле (.rpy)"
                      >
                        <FileCode size={11} />
                        <span>Код</span>
                      </button>
                    {/if}
                    <button
                      class="btn-open-dialogues"
                      onclick={() => onopen(node.scene!.id)}
                      title="Открыть диалоги сцены в таблице"
                    >
                      Диалоги →
                    </button>
                  </div>
                {:else}
                  <div class="node-footer-btns ml-auto">
                    {#if onviewcode}
                      <button
                        type="button"
                        class="btn-code-node"
                        onclick={(e) => { e.stopPropagation(); onviewcode?.(node.scene!); }}
                        title="Посмотреть реализацию развилки/меню в коде (.rpy)"
                      >
                        <FileCode size={11} />
                        <span>Код</span>
                      </button>
                    {/if}
                    <span class="node-badge-router" title="Метка содержит меню или переходы, но не имеет диалоговых строк">
                      Развилка / Меню
                    </span>
                  </div>
                {/if}
              </div>
            {:else}
              <div class="node-footer external-footer" class:external-busy={isThisNodeBusy}>
                {#if isThisNodeBusy}
                  <span class="external-search-pill">
                    <Loader2 size={11} class="spin" />
                    <span>Поиск...</span>
                  </span>
                {:else}
                  <span>Другой файл проекта</span>
                {/if}
              </div>
            {/if}
          </div>
        {/each}

        <!-- Слой плашек-лейблов переходов (над карточками) -->
        <svg class="graph-labels-svg" width={graph.width} height={graph.height} aria-label="Метки переходов">
          {#each graph.links as edge (edge.id)}
            {@const isRelated = edge.from === selected?.name || edge.to === selected?.name}
            {@const isChoice = edge.kind === 'choice' || edge.kind === 'branch'}
            {@const isSelf = edge.from === edge.to}
            {@const cleanText = edge.condition
              ? edge.condition.replace(/^Выбор:\s*"?/, '').replace(/"?$/, '').trim()
              : edge.kind}
            {@const fullText = isSelf
              ? `↺ ${cleanText}`
              : isChoice
                ? `◆ ${cleanText}`
                : cleanText}
            {@const displayText = fullText.length > 17 ? fullText.slice(0, 16) + '…' : fullText}
            {@const pillWidth = Math.min(136, Math.max(36, Math.round(displayText.length * 6.4 + 16)))}
            <g
              class="edge-label-group"
              class:related={isRelated}
              class:branch={isChoice}
              class:self-loop={isSelf}
              transform="translate({edge.labelX}, {edge.labelY})"
            >
              <rect
                x={-pillWidth / 2}
                y={-10}
                width={pillWidth}
                height={20}
                rx="10"
                ry="10"
                class="label-badge"
              />
              <text x="0" y="3.5" text-anchor="middle" class="label-text">
                {displayText}
              </text>
              <title>{edge.from} → {edge.to} ({edge.condition || edge.kind})</title>
            </g>
          {/each}
        </svg>
      </div>
    </div>
  </div>
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
    justify-content: space-between;
    gap: 12px;
    height: 42px;
    min-height: 42px;
    max-height: 42px;
    box-sizing: border-box;
    padding: 0 14px;
    background: #0e1219;
    border-bottom: 1px solid #1a2230;
    flex-shrink: 0;
  }

  .btn-sidebar-burger {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    min-width: 24px;
    min-height: 24px;
    max-width: 24px;
    max-height: 24px;
    box-sizing: border-box;
    padding: 0;
    margin: 0;
    background: #141923;
    border: 1px solid #1e2636;
    border-radius: 4px;
    color: #94a3b8;
    cursor: pointer;
    transition: all 0.15s ease;
    flex-shrink: 0;
  }

  .btn-sidebar-burger :global(svg) {
    display: block;
    flex-shrink: 0;
  }

  .btn-sidebar-burger:hover {
    background: #1e293b;
    border-color: #38bdf8;
    color: #f1f5f9;
  }

  .btn-graph-back {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 5px;
    height: 22px;
    min-height: 22px;
    max-height: 22px;
    padding: 0 8px;
    background: #18202d;
    border: 1px solid #232d3f;
    border-radius: 4px;
    color: #38bdf8;
    font-family: inherit;
    font-size: 11px;
    font-weight: 500;
    line-height: 1;
    box-sizing: border-box;
    cursor: pointer;
    margin: 0;
    transition: all 0.12s;
  }

  .btn-graph-back:hover:not(:disabled) {
    background: #1f2b3e;
    border-color: #38bdf8;
    color: #f0f9ff;
  }

  .btn-graph-back:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .toolbar-info {
    display: flex;
    align-items: center;
    gap: 10px;
    flex: 1;
    min-width: 0;
    font-size: 11px;
    color: #94a3b8;
  }

  .subtoolbar-divider {
    width: 1px;
    height: 18px;
    background: #1e2636;
    margin: 0 4px;
    flex-shrink: 0;
  }

  .info-stat {
    font-size: 11px;
    color: #94a3b8;
    white-space: nowrap;
  }

  .info-stat strong {
    color: #f1f5f9;
    font-weight: 600;
  }

  .zoom-controls {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }

  .zoom-controls button {
    height: 26px;
    min-height: 26px;
    max-height: 26px;
    box-sizing: border-box;
    padding: 0 8px;
    font-size: 11.5px;
    line-height: 1;
    display: inline-flex;
    align-items: center;
    justify-content: center;
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
    top: 20px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    z-index: 10;
  }

  .banner-top-row {
    display: flex;
    justify-content: center;
    width: 100%;
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

  .column-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
    width: 100%;
    padding: 5px 8px;
    background: #0b1320;
    border: 1px solid #1e293b;
    border-radius: 8px;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.4);
  }

  .col-search-box {
    position: relative;
    display: flex;
    align-items: center;
    flex: 1;
    min-width: 0;
  }

  .col-search-input {
    width: 100%;
    padding: 3px 20px 3px 7px;
    background: #131d2d;
    border: 1px solid #293548;
    border-radius: 5px;
    color: #e2e8f0;
    font-size: 11px;
    outline: none;
    transition: border-color 0.15s;
  }

  .col-search-input:focus {
    border-color: #38bdf8;
  }

  .col-clear-btn {
    position: absolute;
    right: 5px;
    background: transparent;
    border: none;
    color: #94a3b8;
    cursor: pointer;
    font-size: 11px;
    padding: 0;
  }

  .col-clear-btn:hover {
    color: #f1f5f9;
  }

  .col-pager {
    display: flex;
    align-items: center;
    gap: 3px;
    flex-shrink: 0;
  }

  .col-page-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    background: #1e293b;
    border: 1px solid #334155;
    border-radius: 4px;
    color: #cbd5e1;
    font-size: 10px;
    cursor: pointer;
    transition: all 0.12s;
  }

  .col-page-btn:hover:not(:disabled) {
    background: #334155;
    color: #38bdf8;
    border-color: #0284c7;
  }

  .col-page-btn:disabled {
    opacity: 0.35;
    cursor: not-allowed;
  }

  .col-page-num {
    font-size: 10.5px;
    font-weight: 600;
    color: #94a3b8;
    min-width: 32px;
    text-align: center;
    font-variant-numeric: tabular-nums;
  }

  .page-size-picker {
    display: flex;
    align-items: center;
    gap: 3px;
    margin-right: 2px;
    height: 26px;
    box-sizing: border-box;
    padding: 0 6px;
    background: #0f172a;
    border: 1px solid #1e293b;
    border-radius: 6px;
  }

  .page-size-label {
    font-size: 11px;
    color: #64748b;
  }

  .page-size-btn {
    height: 20px;
    padding: 0 5px;
    box-sizing: border-box;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    line-height: 1;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 4px;
    color: #94a3b8;
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
  }

  .page-size-btn.size-active {
    background: #1e293b;
    border-color: #0284c7;
    color: #38bdf8;
  }

  .col-reset-btn {
    margin-top: 8px;
    padding: 4px 10px;
    background: #1e293b;
    border: 1px solid #334155;
    border-radius: 6px;
    color: #38bdf8;
    font-size: 11px;
    cursor: pointer;
  }

  .col-reset-btn:hover {
    background: #334155;
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
  .graph-connections-svg {
    position: absolute;
    inset: 0;
    overflow: visible;
    pointer-events: none;
    z-index: 1;
  }

  .graph-labels-svg {
    position: absolute;
    inset: 0;
    overflow: visible;
    pointer-events: none;
    z-index: 15;
  }

  .edge-path-group {
    stroke: #475569;
  }

  .edge-path-group.branch {
    stroke: #8b5cf6;
  }

  .edge-path-group.related {
    stroke: #0284c7;
  }

  .edge-label-group {
    stroke: none;
    pointer-events: auto;
    cursor: default;
  }

  .label-badge {
    fill: #182232;
    stroke: #475569;
    stroke-width: 1.2px;
    filter: drop-shadow(0 2px 4px rgba(0, 0, 0, 0.5));
    transition: fill 0.15s, stroke 0.15s;
  }

  .label-text {
    fill: #cbd5e1;
    stroke: none;
    font: 10px system-ui, -apple-system, sans-serif;
    font-weight: 600;
    pointer-events: none;
    user-select: none;
    transition: fill 0.15s;
  }

  /* Choice / Branch (Purple) - clearly distinguished from blue node */
  .edge-label-group.branch .label-badge {
    fill: #22123b;
    stroke: #8b5cf6;
  }
  .edge-label-group.branch .label-text {
    fill: #f3e8ff;
    font-weight: 600;
  }

  /* Related Choice (Highlighted Purple) - stays purple, does not turn blue */
  .edge-label-group.branch.related .label-badge {
    fill: #2e104f;
    stroke: #c084fc;
    stroke-width: 1.5px;
    filter: drop-shadow(0 0 6px rgba(168, 85, 247, 0.45));
  }
  .edge-label-group.branch.related .label-text {
    fill: #ffffff;
    font-weight: 700;
  }

  /* Self-loop (Cyan) */
  .edge-label-group.self-loop .label-badge {
    fill: #0c2744;
    stroke: #0284c7;
  }
  .edge-label-group.self-loop .label-text {
    fill: #7dd3fc;
  }
  .edge-label-group.self-loop.related .label-badge {
    fill: #0e375e;
    stroke: #38bdf8;
    stroke-width: 1.5px;
  }
  .edge-label-group.self-loop.related .label-text {
    fill: #ffffff;
    font-weight: 700;
  }

  /* Jump / Condition Related (Slate, NOT cyan) */
  .edge-label-group.related:not(.branch):not(.self-loop) .label-badge {
    fill: #222f42;
    stroke: #94a3b8;
    stroke-width: 1.5px;
    filter: drop-shadow(0 2px 6px rgba(0, 0, 0, 0.6));
  }
  .edge-label-group.related:not(.branch):not(.self-loop) .label-text {
    fill: #ffffff;
    font-weight: 700;
  }

  .edge-label-group:hover .label-badge {
    stroke: #f8fafc;
  }
  .edge-label-group:hover .label-text {
    fill: #ffffff;
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
    z-index: 2;
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
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: #f1f5f9;
    font-size: 13px;
  }

  .node-name span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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
    min-height: 31px;
    box-sizing: border-box;
  }

  .node-stats {
    color: #64748b;
    font-size: 10.5px;
  }

  .node-badge-router {
    margin-left: auto;
    padding: 2px 7px;
    border-radius: 4px;
    background: #1e1b4b;
    border: 1px solid #4338ca;
    color: #a5b4fc;
    font-size: 10px;
    font-weight: 600;
  }

  .toolbar-scene-heading {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  .focus-title-badge {
    color: #94a3b8;
    font-size: 11.5px;
    font-weight: 500;
  }

  .focus-scene-name {
    color: #f1f5f9;
    font-size: 13px;
    font-weight: 700;
  }

  .btn-toolbar-code {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 3px;
    height: 22px;
    min-height: 22px;
    max-height: 22px;
    padding: 0 7px;
    font-family: inherit;
    font-size: 11px;
    font-weight: 500;
    line-height: 1;
    background: #111a2e;
    border: 1px solid #273549;
    border-radius: 4px;
    color: #94a3b8;
    cursor: pointer;
    box-sizing: border-box;
    transition: all 0.15s;
  }

  .btn-toolbar-code:hover {
    background: #1e293b;
    border-color: #0284c7;
    color: #38bdf8;
  }

  .node-footer-btns {
    display: flex;
    align-items: center;
    gap: 5px;
  }

  .ml-auto {
    margin-left: auto;
  }

  .btn-code-node {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 3px;
    height: 22px;
    min-height: 22px;
    max-height: 22px;
    padding: 0 7px;
    font-family: inherit;
    font-size: 11px;
    font-weight: 500;
    line-height: 1;
    background: #111a2e;
    border: 1px solid #273549;
    border-radius: 4px;
    color: #94a3b8;
    cursor: pointer;
    box-sizing: border-box;
    transition: all 0.15s;
  }

  .btn-code-node:hover {
    background: #1e293b;
    border-color: #0284c7;
    color: #38bdf8;
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

  .external-footer.external-busy {
    color: #38bdf8;
  }

  .external-search-pill {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    color: #38bdf8;
    font-weight: 600;
  }

  .node.external.navigating {
    border-color: #38bdf8 !important;
    background: #091a2e !important;
    box-shadow: 0 0 18px rgba(56, 189, 248, 0.35) !important;
  }

  .node-main:disabled {
    opacity: 0.85;
    cursor: wait;
  }

  .node-main.is-navigating:disabled {
    opacity: 1;
    cursor: wait;
  }

  .empty {
    padding: 24px;
    color: #64748b;
    text-align: center;
  }
</style>
