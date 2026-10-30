export interface GraphScene {
  id: string;
  name: string;
  total_lines: number;
  translated_lines: number;
  choices: { prompt: string | null; options: { text: string; target_jump: string | null }[] }[];
  jumps: string[];
  entries: { original_text: string }[];
}

export interface GraphEdge {
  from: string;
  to: string;
  kind: string;
  condition: string | null;
}

export interface GraphNode {
  id: string;
  name: string;
  scene?: GraphScene;
  x: number;
  y: number;
  role?: 'center' | 'incoming' | 'outgoing';
}

export interface GraphLink extends GraphEdge {
  id: number;
  path: string;
  labelX: number;
  labelY: number;
}

export interface GraphData {
  nodes: GraphNode[];
  links: GraphLink[];
  width: number;
  height: number;
  focusName?: string;
  incomingCount: number;
  outgoingCount: number;
}

export const NODE_WIDTH = 270;
export const NODE_HEIGHT = 140;
export const GAP_X = 140;
export const GAP_Y = 24;

export function collectAllEdges(scenes: GraphScene[], sourceEdges: GraphEdge[]): GraphEdge[] {
  const edges = [...sourceEdges];
  for (const scene of scenes) {
    for (const choice of scene.choices ?? []) {
      for (const option of choice.options) {
        if (
          option.target_jump &&
          !edges.some(
            e => e.from === scene.name && e.to === option.target_jump && e.condition?.includes(option.text)
          )
        ) {
          edges.push({ from: scene.name, to: option.target_jump, kind: 'choice', condition: option.text });
        }
      }
    }
    for (const target of scene.jumps ?? []) {
      if (!edges.some(e => e.from === scene.name && e.to === target)) {
        edges.push({ from: scene.name, to: target, kind: 'jump', condition: null });
      }
    }
  }
  return edges.filter(
    (edge, i) =>
      edges.findIndex(
        e =>
          e.from === edge.from &&
          e.to === edge.to &&
          e.kind === edge.kind &&
          e.condition === edge.condition
      ) === i
  );
}

/**
 * Строит локальный сфокусированный граф сцены:
 * Слева: входящие узлы (откуда пришли)
 * По центру: текущая выбранная сцена
 * Справа: исходящие развилки и переходы (куда переходим)
 */
export function buildFocusedSceneGraph(
  scenes: GraphScene[],
  sourceEdges: GraphEdge[],
  focusName?: string | null
): GraphData {
  if (scenes.length === 0) {
    return { nodes: [], links: [], width: 600, height: 400, incomingCount: 0, outgoingCount: 0 };
  }

  const allEdges = collectAllEdges(scenes, sourceEdges);
  const sceneMap = new Map(scenes.map(s => [s.name, s]));

  // Определяем центральную сцену
  const targetScene = (focusName && sceneMap.get(focusName)) || scenes[0];
  const centerName = targetScene.name;

  // Входящие ребра (кроме self-loops)
  const incomingEdges = allEdges.filter(e => e.to === centerName && e.from !== centerName);
  const incomingNames = Array.from(new Set(incomingEdges.map(e => e.from)));

  // Исходящие ребра (кроме self-loops)
  const outgoingEdges = allEdges.filter(e => e.from === centerName && e.to !== centerName);
  const outgoingNames = Array.from(new Set(outgoingEdges.map(e => e.to)));

  // Петли в себя
  const selfLoops = allEdges.filter(e => e.from === centerName && e.to === centerName);

  const numIn = incomingNames.length;
  const numOut = outgoingNames.length;

  const heightIn = numIn > 0 ? numIn * NODE_HEIGHT + (numIn - 1) * GAP_Y : 0;
  const heightOut = numOut > 0 ? numOut * NODE_HEIGHT + (numOut - 1) * GAP_Y : 0;
  const maxContentHeight = Math.max(NODE_HEIGHT, heightIn, heightOut);

  const startX = 60;
  const centerX = startX + NODE_WIDTH + GAP_X;
  const rightX = centerX + NODE_WIDTH + GAP_X;

  const topPadding = 90;
  const midY = topPadding + maxContentHeight / 2;

  // Центральный узел
  const centerNode: GraphNode = {
    id: `node-${centerName}`,
    name: centerName,
    scene: sceneMap.get(centerName),
    x: centerX,
    y: midY - NODE_HEIGHT / 2,
    role: 'center',
  };

  // Входящие узлы
  const inStartY = midY - heightIn / 2;
  const incomingNodes: GraphNode[] = incomingNames.map((name, idx) => ({
    id: `node-in-${name}`,
    name,
    scene: sceneMap.get(name),
    x: startX,
    y: inStartY + idx * (NODE_HEIGHT + GAP_Y),
    role: 'incoming',
  }));

  // Исходящие узлы
  const outStartY = midY - heightOut / 2;
  const outgoingNodes: GraphNode[] = outgoingNames.map((name, idx) => ({
    id: `node-out-${name}`,
    name,
    scene: sceneMap.get(name),
    x: rightX,
    y: outStartY + idx * (NODE_HEIGHT + GAP_Y),
    role: 'outgoing',
  }));

  const nodes = [...incomingNodes, centerNode, ...outgoingNodes];
  const links: GraphLink[] = [];

  // Связи: Входящие -> Центр
  const inGroups = new Map<string, GraphEdge[]>();
  for (const edge of incomingEdges) {
    if (!inGroups.has(edge.from)) inGroups.set(edge.from, []);
    inGroups.get(edge.from)!.push(edge);
  }

  for (const fromNode of incomingNodes) {
    const edgesGroup = inGroups.get(fromNode.name) ?? [];
    edgesGroup.forEach((edge, groupIdx) => {
      const offset = (groupIdx - (edgesGroup.length - 1) / 2) * 12;
      const sx = fromNode.x + NODE_WIDTH;
      const sy = fromNode.y + NODE_HEIGHT / 2 + offset;
      const tx = centerNode.x;
      const ty = centerNode.y + NODE_HEIGHT / 2 + offset;
      const bend = GAP_X / 2;
      const path = `M ${sx} ${sy} C ${sx + bend} ${sy}, ${tx - bend} ${ty}, ${tx} ${ty}`;
      links.push({
        ...edge,
        id: links.length,
        path,
        labelX: (sx + tx) / 2,
        labelY: (sy + ty) / 2 - 10,
      });
    });
  }

  // Связи: Центр -> Исходящие
  const outGroups = new Map<string, GraphEdge[]>();
  for (const edge of outgoingEdges) {
    if (!outGroups.has(edge.to)) outGroups.set(edge.to, []);
    outGroups.get(edge.to)!.push(edge);
  }

  for (const toNode of outgoingNodes) {
    const edgesGroup = outGroups.get(toNode.name) ?? [];
    edgesGroup.forEach((edge, groupIdx) => {
      const offset = (groupIdx - (edgesGroup.length - 1) / 2) * 12;
      const sx = centerNode.x + NODE_WIDTH;
      const sy = centerNode.y + NODE_HEIGHT / 2 + offset;
      const tx = toNode.x;
      const ty = toNode.y + NODE_HEIGHT / 2 + offset;
      const bend = GAP_X / 2;
      const path = `M ${sx} ${sy} C ${sx + bend} ${sy}, ${tx - bend} ${ty}, ${tx} ${ty}`;
      links.push({
        ...edge,
        id: links.length,
        path,
        labelX: (sx + tx) / 2,
        labelY: (sy + ty) / 2 - 10,
      });
    });
  }

  // Петли в себя (Self-loops)
  for (const edge of selfLoops) {
    const sx = centerNode.x + NODE_WIDTH * 0.75;
    const sy = centerNode.y;
    const tx = centerNode.x + NODE_WIDTH * 0.25;
    const ty = centerNode.y;
    const path = `M ${sx} ${sy} C ${sx + 20} ${sy - 50}, ${tx - 20} ${ty - 50}, ${tx} ${ty}`;
    links.push({
      ...edge,
      id: links.length,
      path,
      labelX: centerNode.x + NODE_WIDTH / 2,
      labelY: centerNode.y - 40,
    });
  }

  const width = rightX + NODE_WIDTH + 80;
  const height = Math.max(480, midY + maxContentHeight / 2 + 80);

  return {
    nodes,
    links,
    width,
    height,
    focusName: centerName,
    incomingCount: numIn,
    outgoingCount: numOut,
  };
}

/**
 * Строит полный топологический граф файла (BFS по слоям)
 */
export function buildFullSceneGraph(scenes: GraphScene[], sourceEdges: GraphEdge[]): GraphData {
  if (scenes.length === 0) {
    return { nodes: [], links: [], width: 600, height: 400, incomingCount: 0, outgoingCount: 0 };
  }

  const uniqueEdges = collectAllEdges(scenes, sourceEdges);
  const sceneMap = new Map(scenes.map(scene => [scene.name, scene]));
  const names = [...sceneMap.keys()];
  for (const edge of uniqueEdges) {
    if (!names.includes(edge.from)) names.push(edge.from);
    if (!names.includes(edge.to)) names.push(edge.to);
  }

  const outgoing = new Map(names.map(name => [name, [] as string[]]));
  const incoming = new Set<string>();
  for (const edge of uniqueEdges) {
    outgoing.get(edge.from)!.push(edge.to);
    incoming.add(edge.to);
  }

  const ranks = new Map<string, number>();
  const rows = new Map<number, number>();
  const roots = [...names.filter(name => !incoming.has(name)), ...names];
  for (const root of roots) {
    if (ranks.has(root)) continue;
    const queue = [root];
    ranks.set(root, 0);
    for (let i = 0; i < queue.length; i++) {
      const name = queue[i];
      for (const target of outgoing.get(name)!) {
        if (!ranks.has(target)) {
          ranks.set(target, ranks.get(name)! + 1);
          queue.push(target);
        }
      }
    }
  }

  const nodes: GraphNode[] = names.map(name => {
    const rank = ranks.get(name) ?? 0;
    const row = rows.get(rank) ?? 0;
    rows.set(rank, row + 1);
    return {
      id: `node-${name}`,
      name,
      scene: sceneMap.get(name),
      x: 70 + rank * (NODE_WIDTH + GAP_X),
      y: 70 + row * (NODE_HEIGHT + GAP_Y + 40),
    };
  });

  const byName = new Map(nodes.map(node => [node.name, node]));
  const links: GraphLink[] = uniqueEdges.map((edge, index) => {
    const from = byName.get(edge.from)!;
    const to = byName.get(edge.to)!;
    const sx = from.x + NODE_WIDTH;
    const sy = from.y + NODE_HEIGHT / 2;
    const tx = to.x;
    const ty = to.y + NODE_HEIGHT / 2;
    const lane = 32 + (index % 5) * 14;
    const forward = tx > sx;
    const bend = Math.max(60, (tx - sx) / 2);
    const path = forward
      ? `M ${sx} ${sy} C ${sx + bend} ${sy}, ${tx - bend} ${ty}, ${tx} ${ty}`
      : `M ${sx} ${sy} C ${sx + lane} ${sy}, ${sx + lane} ${from.y - lane}, ${sx} ${from.y - lane} L ${tx - lane} ${from.y - lane} C ${tx - lane * 2} ${from.y - lane}, ${tx - lane} ${ty}, ${tx} ${ty}`;
    return {
      ...edge,
      id: index,
      path,
      labelX: forward ? (sx + tx) / 2 : from.x + NODE_WIDTH / 2,
      labelY: forward ? (sy + ty) / 2 - 10 : from.y - lane - 8,
    };
  });

  return {
    nodes,
    links,
    width: Math.max(600, ...nodes.map(n => n.x + NODE_WIDTH + 100)),
    height: Math.max(400, ...nodes.map(n => n.y + NODE_HEIGHT + 70)),
    incomingCount: 0,
    outgoingCount: 0,
  };
}

/**
 * Основная точка входа построения графа
 */
export function buildSceneGraph(
  scenes: GraphScene[],
  sourceEdges: GraphEdge[],
  options?: { mode?: 'focused' | 'all'; focusName?: string | null }
): GraphData {
  if (options?.mode === 'focused') {
    return buildFocusedSceneGraph(scenes, sourceEdges, options.focusName);
  }
  return buildFullSceneGraph(scenes, sourceEdges);
}
