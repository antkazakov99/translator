import type { FileScenesDto_Serialize, SceneItemDto_Serialize } from "./bindings";

/**
 * Список сцен таблицы перевода: сцены по файлам, у каждого файла «Без сцены» последней. Поиск идёт по
 * названию и метке сцены и по пути файла: в больших играх сотни сцен.
 */

export type FileScenes = FileScenesDto_Serialize;
export type SceneItem = SceneItemDto_Serialize;

/** Открытая в таблице сцена или реплики файла вне сцен. */
export type ScenePick = { kind: "scene"; key: string } | { kind: "loose"; file: number };

/** Сколько сцен показывает поиск: остальные находятся точнее. */
export const SHOWN = 300;

/** Подпись псевдосцены с репликами вне сцен. */
export const LOOSE_NAME = "Без сцены";

/** Строковый ключ выбора: по нему сравнивают выбранное и пункт списка. */
export function pickId(pick: ScenePick): string {
  return pick.kind === "scene" ? `scene:${pick.key}` : `loose:${pick.file}`;
}

/** Метка сцены без вида ключа: `label:start` → `start`. */
export const sceneLabel = (key: string) => key.replace(/^[a-z]+:/, "");

/** «Название · метка», метка без названия или «Без сцены». */
export function sceneItemName(scene: Pick<SceneItem, "key" | "title">): string {
  if (!scene.key) return LOOSE_NAME;
  const label = sceneLabel(scene.key);
  return scene.title ? `${scene.title} · ${label}` : label;
}

export interface TreeView {
  groups: { group: FileScenes; scenes: SceneItem[] }[];
  /** Сцены, которые подошли поиску, но не показаны. */
  hidden: number;
}

/**
 * Группы и сцены для показа. Без поиска все группы со всеми сценами. С поиском файл, путь которого подошёл,
 * показывается целиком, у остальных только подошедшие сцены, пустые группы пропадают. Показывается не больше
 * `shown` сцен.
 */
export function filterTree(tree: FileScenes[], query: string, shown = SHOWN): TreeView {
  const needle = query.trim().toLowerCase();
  if (!needle) return { groups: tree.map((group) => ({ group, scenes: group.scenes })), hidden: 0 };
  const has = (text: string | null | undefined) => !!text && text.toLowerCase().includes(needle);
  const groups: TreeView["groups"] = [];
  let left = shown;
  let hidden = 0;
  for (const group of tree) {
    const whole = has(group.file.path);
    const found = whole
      ? group.scenes
      : group.scenes.filter((scene) => (scene.key ? has(scene.title) || has(sceneLabel(scene.key)) : has(LOOSE_NAME)));
    const taken = found.slice(0, Math.max(left, 0));
    hidden += found.length - taken.length;
    left -= taken.length;
    if (taken.length > 0) groups.push({ group, scenes: taken });
  }
  return { groups, hidden };
}
