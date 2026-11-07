import type { GraphEdgeDto, GraphSceneDto_Serialize } from "./bindings";

/**
 * Раскладка графа сцен вокруг выбранной: слева сцены, из которых в неё приходят, справа те, куда из неё
 * уходят. Весь граф игры на экран не помещается: у сцен-хабов бывают тысячи переходов, поэтому колонки
 * листаются страницами и ищутся по имени и названию.
 */

export type GraphScene = GraphSceneDto_Serialize;
export type GraphEdge = GraphEdgeDto;

export interface GraphNode {
  key: string;
  /** Нет у сцены, которой нет в снимке: такой конец перехода показывается без прогресса. */
  scene?: GraphScene;
  x: number;
  y: number;
  role: "center" | "incoming" | "outgoing";
}

/**
 * Линия между двумя сценами. Все переходы пары рисуются одной линией: у сцен-хабов между двумя
 * сценами бывают десятки условий, и отдельные линии с подписями слипались бы.
 */
export interface GraphLink {
  id: number;
  from: string;
  to: string;
  /** Вид перехода, если он у всех переходов пары один. Иначе `mixed`. */
  kind: string;
  edges: GraphEdge[];
  path: string;
  /** Подпись у внешней сцены: так подписи идут по строкам карточек и не наезжают друг на друга. */
  label: string;
  labelX: number;
  labelY: number;
  labelAnchor: "start" | "middle" | "end";
  /** Все переходы пары для подсказки. */
  title: string;
}

export interface Column {
  /** Сцен в колонке без фильтра. */
  total: number;
  /** Сцен после фильтра. */
  found: number;
  pages: number;
  page: number;
}

export interface GraphLayout {
  nodes: GraphNode[];
  links: GraphLink[];
  width: number;
  height: number;
  center: string | null;
  incoming: Column;
  outgoing: Column;
}

export interface LayoutOptions {
  focus?: string | null;
  pageSize?: number;
  incomingPage?: number;
  incomingFilter?: string;
  outgoingPage?: number;
  outgoingFilter?: string;
}

export const NODE_WIDTH = 260;
export const NODE_HEIGHT = 112;
export const GAP_X = 180;
export const GAP_Y = 20;
/** Место над колонками под их заголовки и поиск. */
const TOP = 96;
const LEFT = 40;

/** Имя сцены без вида ключа: `label:day1` → `day1`. */
export const sceneLabel = (key: string) => key.replace(/^[a-z]+:/, "");

/** Подпись сцены: название, если есть, и имя метки. */
export function sceneName(key: string, scene?: GraphScene) {
  const label = sceneLabel(key);
  return scene?.title ? `${scene.title} · ${label}` : label;
}

const emptyColumn = (): Column => ({ total: 0, found: 0, pages: 1, page: 1 });

/** Соседи по одну сторону: без повторов, фильтр по имени и названию, страница. */
function column(keys: string[], scenes: Map<string, GraphScene>, filter: string, page: number, size: number) {
  const unique = [...new Set(keys)];
  const query = filter.trim().toLowerCase();
  const found = query
    ? unique.filter((key) => sceneName(key, scenes.get(key)).toLowerCase().includes(query))
    : unique;
  const pages = Math.max(1, Math.ceil(found.length / size));
  const current = Math.min(Math.max(1, page), pages);
  return {
    shown: found.slice((current - 1) * size, current * size),
    info: { total: unique.length, found: found.length, pages, page: current } satisfies Column,
  };
}

/** Кривая от правого края одного узла к левому краю другого. */
function curve(sx: number, sy: number, tx: number, ty: number) {
  const bend = GAP_X / 2;
  return `M ${sx} ${sy} C ${sx + bend} ${sy}, ${tx - bend} ${ty}, ${tx} ${ty}`;
}

export function layoutGraph(scenes: GraphScene[], edges: GraphEdge[], options: LayoutOptions = {}): GraphLayout {
  const byKey = new Map(scenes.map((scene) => [scene.key, scene]));
  const center = (options.focus && byKey.has(options.focus) ? options.focus : scenes[0]?.key) ?? null;
  if (!center) {
    return { nodes: [], links: [], width: 600, height: 400, center: null, incoming: emptyColumn(), outgoing: emptyColumn() };
  }

  const size = Math.max(1, options.pageSize ?? 10);
  const into = edges.filter((edge) => edge.to === center && edge.from !== center);
  const out = edges.filter((edge) => edge.from === center && edge.to !== center);
  const loops = edges.filter((edge) => edge.from === center && edge.to === center);
  const left = column(into.map((edge) => edge.from), byKey, options.incomingFilter ?? "", options.incomingPage ?? 1, size);
  const right = column(out.map((edge) => edge.to), byKey, options.outgoingFilter ?? "", options.outgoingPage ?? 1, size);

  const columnHeight = (count: number) => (count > 0 ? count * NODE_HEIGHT + (count - 1) * GAP_Y : 0);
  const contentHeight = Math.max(NODE_HEIGHT, columnHeight(left.shown.length), columnHeight(right.shown.length));
  const middle = TOP + contentHeight / 2;
  const centerX = LEFT + NODE_WIDTH + GAP_X;
  const rightX = centerX + NODE_WIDTH + GAP_X;

  const centerNode: GraphNode = { key: center, scene: byKey.get(center), x: centerX, y: middle - NODE_HEIGHT / 2, role: "center" };
  const place = (keys: string[], x: number, role: GraphNode["role"]): GraphNode[] => {
    const top = middle - columnHeight(keys.length) / 2;
    return keys.map((key, index) => ({ key, scene: byKey.get(key), x, y: top + index * (NODE_HEIGHT + GAP_Y), role }));
  };
  const incoming = place(left.shown, LEFT, "incoming");
  const outgoing = place(right.shown, rightX, "outgoing");

  const links: GraphLink[] = [];
  const link = (group: GraphEdge[], path: string, label: { x: number; y: number; anchor: GraphLink["labelAnchor"] }) => {
    const kinds = new Set(group.map((edge) => edge.kind));
    links.push({
      id: links.length,
      from: group[0].from,
      to: group[0].to,
      kind: kinds.size === 1 ? group[0].kind : "mixed",
      edges: group,
      path,
      label: group.length === 1 ? edgeLabel(group[0]) : `переходов: ${group.length}`,
      labelX: label.x,
      labelY: label.y,
      labelAnchor: label.anchor,
      title: group.map((edge) => edgeLabel(edge) || "переход").join("\n"),
    });
  };
  // У центра концы линий расходятся по высоте карточки, у соседей входят в середину.
  const spread = (index: number, count: number) => centerNode.y + (NODE_HEIGHT * (index + 1)) / (count + 1);
  incoming.forEach((node, index) => {
    const sx = node.x + NODE_WIDTH;
    const sy = node.y + NODE_HEIGHT / 2;
    const path = curve(sx, sy, centerNode.x - 2, spread(index, incoming.length));
    link(into.filter((edge) => edge.from === node.key), path, { x: sx + 8, y: sy - 8, anchor: "start" });
  });
  outgoing.forEach((node, index) => {
    const ty = node.y + NODE_HEIGHT / 2;
    const path = curve(centerNode.x + NODE_WIDTH, spread(index, outgoing.length), node.x - 2, ty);
    link(out.filter((edge) => edge.to === node.key), path, { x: node.x - 8, y: ty - 8, anchor: "end" });
  });
  if (loops.length > 0) {
    const half = 36;
    const lift = 44;
    const sx = centerNode.x + NODE_WIDTH / 2 + half;
    const tx = centerNode.x + NODE_WIDTH / 2 - half;
    const y = centerNode.y;
    const path = `M ${sx} ${y} C ${sx + 4} ${y - lift}, ${tx - 4} ${y - lift}, ${tx} ${y - 2}`;
    link(loops, path, { x: centerNode.x + NODE_WIDTH / 2, y: y - lift + 4, anchor: "middle" });
  }

  return {
    nodes: [...incoming, centerNode, ...outgoing],
    links,
    width: rightX + NODE_WIDTH + LEFT,
    height: Math.max(420, middle + contentHeight / 2 + 48),
    center,
    incoming: left.info,
    outgoing: right.info,
  };
}

/** Короткая подпись перехода на линии. */
export function edgeLabel(edge: GraphEdge) {
  switch (edge.kind) {
    case "choice":
      return edge.condition ? edge.condition.replace(/^Выбор:\s*/, "") : "выбор";
    case "branch":
      return edge.condition ? `если ${edge.condition}` : "условие";
    case "screen":
      return edge.condition ? `экран ${edge.condition}` : "экран";
    case "call":
      return "call";
    default:
      return "";
  }
}
