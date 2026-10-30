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

export const NODE_WIDTH = 260;
export const NODE_HEIGHT = 150;

// Exact labels only: "end" and "end_scene" are different Ren'Py targets.
export function buildSceneGraph(scenes: GraphScene[], sourceEdges: GraphEdge[]) {
  const edges = [...sourceEdges];
  for (const scene of scenes) {
    for (const choice of scene.choices ?? []) {
      for (const option of choice.options) {
        if (option.target_jump && !edges.some(e => e.from === scene.name && e.to === option.target_jump && e.condition?.includes(option.text))) {
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
  const uniqueEdges = edges.filter((edge, i) => edges.findIndex(e => e.from === edge.from && e.to === edge.to && e.kind === edge.kind && e.condition === edge.condition) === i);
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
  // Breadth-first placement terminates for cycles, including self-loops.
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
  const nodes = names.map(name => {
    const rank = ranks.get(name)!;
    const row = rows.get(rank) ?? 0;
    rows.set(rank, row + 1);
    return { name, scene: sceneMap.get(name), x: 70 + rank * 420, y: 70 + row * 230 };
  });
  const byName = new Map(nodes.map(node => [node.name, node]));
  const links = uniqueEdges.map((edge, index) => {
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
    return { ...edge, id: index, path, labelX: forward ? (sx + tx) / 2 : from.x + NODE_WIDTH / 2, labelY: forward ? (sy + ty) / 2 - 10 : from.y - lane - 8 };
  });
  return { nodes, links, width: Math.max(600, ...nodes.map(n => n.x + NODE_WIDTH + 100)), height: Math.max(400, ...nodes.map(n => n.y + NODE_HEIGHT + 70)) };
}
