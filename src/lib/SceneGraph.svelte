<script lang="ts">
  import { tick } from 'svelte';
  import { buildSceneGraph, NODE_WIDTH, NODE_HEIGHT, type GraphScene, type GraphEdge } from './scene-graph';

  let { scenes, edges, selectedId, onselect, onnavigate, onopen, busy = false }: {
    scenes: GraphScene[]; edges: GraphEdge[]; selectedId: string;
    onselect: (scene: GraphScene) => void; onnavigate: (name: string) => void;
    onopen: (id: string) => void; busy?: boolean;
  } = $props();
  let graph = $derived(buildSceneGraph(scenes, edges));
  let selected = $derived(scenes.find(scene => scene.id === selectedId));
  let zoom = $state(1);
  let viewport: HTMLDivElement;
  let drag: { x: number; y: number; left: number; top: number } | null = null;
  let dragging = $state(false);
  let links = $derived(graph.links.filter(edge => edge.from === selected?.name || edge.to === selected?.name));

  function focus(name: string) {
    const node = graph.nodes.find(node => node.name === name);
    if (!node || !viewport) return;
    viewport.scrollTo({ left: (node.x + NODE_WIDTH / 2) * zoom - viewport.clientWidth / 2, top: (node.y + NODE_HEIGHT / 2) * zoom - viewport.clientHeight / 2, behavior: 'smooth' });
  }
  $effect(() => {
    const name = selected?.name;
    if (name) void tick().then(() => focus(name));
  });
  async function setZoom(value: number) {
    const old = zoom;
    const x = (viewport.scrollLeft + viewport.clientWidth / 2) / old;
    const y = (viewport.scrollTop + viewport.clientHeight / 2) / old;
    zoom = Math.max(0.15, Math.min(2, value));
    await tick();
    viewport.scrollLeft = x * zoom - viewport.clientWidth / 2;
    viewport.scrollTop = y * zoom - viewport.clientHeight / 2;
  }
  async function fit() {
    zoom = Math.max(0.15, Math.min(1, viewport.clientWidth / graph.width, viewport.clientHeight / graph.height));
    await tick();
    viewport.scrollTo(0, 0);
  }
  function startPan(event: PointerEvent) {
    if (event.button !== 0 || (event.target as Element).closest('button')) return;
    drag = { x: event.clientX, y: event.clientY, left: viewport.scrollLeft, top: viewport.scrollTop };
    dragging = true;
    viewport.setPointerCapture(event.pointerId);
  }
  function pan(event: PointerEvent) {
    if (!drag) return;
    viewport.scrollLeft = drag.left - (event.clientX - drag.x);
    viewport.scrollTop = drag.top - (event.clientY - drag.y);
  }
  function endPan() { drag = null; dragging = false; }
</script>

<div class="graph-view">
  <div class="toolbar">
    <span>{graph.nodes.length} узлов · {graph.links.length} связей</span>
    <button onclick={() => setZoom(zoom / 1.2)} aria-label="Уменьшить масштаб">−</button>
    <button onclick={() => setZoom(1)} title="Масштаб 100%">{Math.round(zoom * 100)}%</button>
    <button onclick={() => setZoom(zoom * 1.2)} aria-label="Увеличить масштаб">+</button>
    <button onclick={fit}>Весь граф</button>
    {#if selected}<button onclick={() => focus(selected.name)}>К выбранной сцене</button>{/if}
  </div>
  <div class="hint">Перетаскивайте фон · Ctrl + колесо — масштаб · Выберите узел для просмотра связей</div>
  <div class="viewport" class:dragging bind:this={viewport} role="application" aria-label="Граф сценария. Используйте стрелки для перемещения" tabindex="0"
    onkeydown={(event) => {
      if (event.target !== viewport) return;
      const offsets: Record<string, [number, number]> = { ArrowLeft: [-80, 0], ArrowRight: [80, 0], ArrowUp: [0, -80], ArrowDown: [0, 80] };
      const offset = offsets[event.key];
      if (offset) { event.preventDefault(); viewport.scrollBy(offset[0], offset[1]); }
    }}
    onpointerdown={startPan} onpointermove={pan} onpointerup={endPan} onpointercancel={endPan} onlostpointercapture={endPan}
    onwheel={(event) => { if (event.ctrlKey) { event.preventDefault(); void setZoom(zoom * (event.deltaY > 0 ? 0.9 : 1.1)); } }}>
    {#if graph.nodes.length === 0}<p class="empty">В этом файле нет сцен для построения графа.</p>{/if}
    <div class="extent" style:width={`${graph.width * zoom}px`} style:height={`${graph.height * zoom}px`}>
      <div class="world" style:width={`${graph.width}px`} style:height={`${graph.height}px`} style:transform={`scale(${zoom})`}>
        <svg width={graph.width} height={graph.height} aria-label="Направленные связи между сценами">
          <defs><marker id="scene-arrow" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto-start-reverse"><path d="M 0 0 L 8 4 L 0 8 z" fill="#94a3b8" /></marker></defs>
          {#each graph.links as edge (edge.id)}
            <g class:related={edge.from === selected?.name || edge.to === selected?.name} class:branch={edge.kind === 'choice' || edge.kind === 'branch'}>
              <path d={edge.path} fill="none" stroke-width="2" marker-end="url(#scene-arrow)" />
              <text x={edge.labelX} y={edge.labelY} text-anchor="middle"><title>{edge.condition || edge.kind}: {edge.from} → {edge.to}</title>{(edge.condition || edge.kind).slice(0, 26)}{(edge.condition?.length ?? 0) > 26 ? '…' : ''}</text>
            </g>
          {/each}
        </svg>
        {#each graph.nodes as node (node.name)}
          <div id={node.scene ? `scene-node-${node.scene.id}` : `scene-node-${node.name}`} class="node" class:external={!node.scene} class:selected={node.scene?.id === selectedId}
            style:left={`${node.x}px`} style:top={`${node.y}px`} style:width={`${NODE_WIDTH}px`} style:height={`${NODE_HEIGHT}px`}>
            <button class="node-main" disabled={busy && !node.scene} onclick={() => node.scene ? onselect(node.scene) : onnavigate(node.name)} title={node.name}>
              <small>{node.scene ? (node.scene.choices?.length ? '◆ Развилка' : 'Сцена') : '↗ Другой файл / внешняя метка'}</small>
              <strong>{node.name}</strong>
              <span class="preview">{node.scene?.entries[0]?.original_text || (node.scene ? 'Нет реплик' : 'Нажмите, чтобы найти и открыть граф')}</span>
            </button>
            {#if node.scene}
              <div class="node-footer"><span>{node.scene.translated_lines}/{node.scene.total_lines} строк</span><button onclick={() => onopen(node.scene!.id)}>Диалоги →</button></div>
            {:else}<div class="node-footer">{busy ? 'Поиск…' : 'Переход по метке'}</div>{/if}
          </div>
        {/each}
      </div>
    </div>
  </div>
  {#if selected}
    <div class="details">
      <strong>{selected.name}</strong>
      {#if links.length === 0}<span>У сцены нет известных связей.</span>{/if}
      {#each links as edge (edge.id)}
        {@const outgoing = edge.from === selected.name}
        <button disabled={busy} onclick={() => onnavigate(outgoing ? edge.to : edge.from)}>
          {outgoing ? '→' : '←'} {outgoing ? edge.to : edge.from} <small>{edge.condition || edge.kind}</small>
        </button>
      {/each}
      {#each selected.choices ?? [] as choice}
        {#each choice.options.filter(option => !option.target_jump) as option}<span>◆ {option.text} — метка перехода не определена</span>{/each}
      {/each}
    </div>
  {/if}
</div>

<style>
  .graph-view { display: flex; flex-direction: column; height: 100%; min-height: 0; background: #0b1019; color: #cbd5e1; }
  .toolbar { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; padding: 10px; border-bottom: 1px solid #263248; }
  .toolbar span { margin-right: auto; font-size: 12px; }
  button { color: #cbd5e1; background: #182337; border: 1px solid #334155; border-radius: 5px; padding: 5px 9px; cursor: pointer; font: inherit; font-size: 12px; }
  button:hover { border-color: #38bdf8; color: #fff; }
  button:focus-visible { outline: 2px solid #38bdf8; outline-offset: 2px; }
  button:disabled { opacity: .5; cursor: wait; }
  .hint { padding: 6px 12px; color: #94a3b8; font-size: 11px; }
  .viewport { flex: 1; min-height: 200px; overflow: auto; cursor: grab; touch-action: none; background-image: radial-gradient(#263248 1px, transparent 1px); background-size: 20px 20px; }
  .viewport.dragging { cursor: grabbing; user-select: none; }
  .extent, .world { position: relative; }
  .world { transform-origin: 0 0; }
  svg { position: absolute; inset: 0; overflow: visible; }
  g { stroke: #64748b; }
  g.branch { stroke: #a78bfa; }
  g.related { stroke: #38bdf8; }
  text { fill: #cbd5e1; stroke: #0b1019; stroke-width: 4px; paint-order: stroke; font: 11px system-ui; }
  .node { position: absolute; box-sizing: border-box; border: 1px solid #334155; border-radius: 10px; background: #131d2d; overflow: hidden; display: flex; flex-direction: column; box-shadow: 0 6px 18px #0004; }
  .node.selected { border: 2px solid #38bdf8; box-shadow: 0 0 0 3px #38bdf820; }
  .node.external { border: 1px dashed #fbbf24; background: #242019; }
  .node-main { display: flex; flex-direction: column; gap: 8px; text-align: left; border: 0; border-radius: 0; background: transparent; padding: 12px; flex: 1; min-height: 0; }
  .node-main small { color: #94a3b8; font-size: 10px; }
  .node-main strong { width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: #e2e8f0; }
  .preview { display: -webkit-box; line-clamp: 2; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; color: #94a3b8; font-size: 11px; }
  .node-footer { display: flex; align-items: center; justify-content: space-between; border-top: 1px solid #334155; padding: 6px 10px; font-size: 11px; }
  .node-footer button { padding: 3px 6px; font-size: 11px; }
  .details { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; max-height: 160px; overflow: auto; padding: 12px; border-top: 1px solid #334155; font-size: 12px; }
  .details strong { width: 100%; overflow-wrap: anywhere; }
  .details small { color: #a78bfa; margin-left: 5px; }
  .details button { overflow-wrap: anywhere; text-align: left; }
  .empty { padding: 20px; }
</style>
