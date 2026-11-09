import type { FileDto, FileScenesDto_Serialize, SceneItemDto_Serialize } from "./bindings";

/**
 * Список сцен таблицы перевода: плоский, по файлам и внутри файла по сюжету. Реплики файла вне сцен идут
 * пунктом «Без сцены · файл» после сцен своего файла. Поиск идёт по названию и метке сцены и по пути файла:
 * в больших играх сотни сцен.
 */

export type FileScenes = FileScenesDto_Serialize;
export type SceneItem = SceneItemDto_Serialize;

/** Открытая в таблице сцена или реплики файла вне сцен. */
export type ScenePick = { kind: "scene"; key: string } | { kind: "loose"; file: number };

/** Пункт списка сцен. */
export interface SceneEntry {
  id: string;
  pick: ScenePick;
  scene: SceneItem;
  file: FileDto;
  name: string;
}

/** Подпись псевдосцены с репликами вне сцен. */
export const LOOSE_NAME = "Без сцены";

/** Строковый ключ выбора: по нему сравнивают выбранное и пункт списка. */
export function pickId(pick: ScenePick): string {
  return pick.kind === "scene" ? `scene:${pick.key}` : `loose:${pick.file}`;
}

/** Метка сцены без вида ключа: `label:start` → `start`. */
export const sceneLabel = (key: string) => key.replace(/^[a-z]+:/, "");

/** Путь файла без папки `game/`. */
export const fileName = (path: string) => path.replace(/^game\//, "");

/** «Название · метка», метка без названия или «Без сцены · файл». */
export function sceneItemName(scene: Pick<SceneItem, "key" | "title">, path?: string): string {
  if (!scene.key) return path ? `${LOOSE_NAME} · ${fileName(path)}` : LOOSE_NAME;
  const label = sceneLabel(scene.key);
  return scene.title ? `${scene.title} · ${label}` : label;
}

/** Все пункты списка по порядку файлов. */
export function sceneList(tree: FileScenes[]): SceneEntry[] {
  return tree.flatMap((group) =>
    group.scenes.map((scene) => {
      const pick: ScenePick = scene.key ? { kind: "scene", key: scene.key } : { kind: "loose", file: group.file.id };
      return { id: pickId(pick), pick, scene, file: group.file, name: sceneItemName(scene, group.file.path) };
    }),
  );
}

/** Пункты, подходящие поиску: по названию, метке и пути файла, без учёта регистра. */
export function filterScenes(list: SceneEntry[], query: string): SceneEntry[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return list;
  const has = (text: string | null | undefined) => !!text && text.toLowerCase().includes(needle);
  return list.filter((entry) => has(entry.name) || (entry.scene.key && has(entry.scene.key.replace(/^[a-z]+:/, ""))) || has(entry.file.path));
}
