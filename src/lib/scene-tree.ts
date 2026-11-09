import type { FileDto, FileScenesDto_Serialize, SceneItemDto_Serialize } from "./bindings";

/**
 * Список сцен таблицы перевода: плоский, по алфавиту оригинального имени сцены (метки). Пункт показывает
 * перевод названия (или пометку, что его нет), под ним оригинал. Реплики файла вне сцен идут в конце пунктами «Без сцены · файл».
 * Поиск идёт по названию, метке и пути файла: в больших играх сотни сцен.
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
  /** Полное имя для шапки и поиска: «Название · метка». */
  name: string;
  /** Первая строка пункта: перевод названия, у «Без сцены» сама подпись. `null`: названия ещё нет. */
  title: string | null;
  /** Вторая строка, есть у каждого пункта: оригинальное имя сцены, у «Без сцены» файл. */
  original: string;
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

/** Сравнение имён без учёта регистра, числа внутри сравниваются как числа: `day2` раньше `day10`. */
const byName = new Intl.Collator("en", { numeric: true, sensitivity: "base" }).compare;

/** Все пункты списка: сцены по алфавиту оригинального имени, за ними «Без сцены» по имени файла. */
export function sceneList(tree: FileScenes[]): SceneEntry[] {
  const entries = tree.flatMap((group) =>
    group.scenes.map((scene): SceneEntry => {
      const pick: ScenePick = scene.key ? { kind: "scene", key: scene.key } : { kind: "loose", file: group.file.id };
      const name = sceneItemName(scene, group.file.path);
      const label = scene.key ? sceneLabel(scene.key) : null;
      // Две строки у каждого пункта: при смеси переведённых и непереведённых пункты одной высоты.
      const title = label === null ? LOOSE_NAME : scene.title;
      const original = label ?? fileName(group.file.path);
      return { id: pickId(pick), pick, scene, file: group.file, name, title, original };
    }),
  );
  const order = (entry: SceneEntry) => (entry.scene.key ? sceneLabel(entry.scene.key) : fileName(entry.file.path));
  return entries.sort((a, b) => {
    const loose = Number(a.pick.kind === "loose") - Number(b.pick.kind === "loose");
    return loose !== 0 ? loose : byName(order(a), order(b));
  });
}

/**
 * Пункты, подходящие поиску: по названию, метке и пути файла без учёта регистра, а также пункты, где поиск
 * по репликам (`search_scenes`) нашёл совпадения. `hits` это число найденных реплик по `SceneEntry.id`.
 */
export function filterScenes(list: SceneEntry[], query: string, hits: ReadonlyMap<string, number> = new Map()): SceneEntry[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return list;
  const has = (text: string | null | undefined) => !!text && text.toLowerCase().includes(needle);
  return list.filter((entry) => has(entry.name) || has(entry.file.path) || hits.has(entry.id));
}

/** Ключи найденных реплик для `filterScenes`: сцена по ключу, «Без сцены» по файлу. */
export function hitMap(hits: { key: string | null; file: number; lines: number }[]): Map<string, number> {
  return new Map(hits.map((hit) => [pickId(hit.key ? { kind: "scene", key: hit.key } : { kind: "loose", file: hit.file }), hit.lines]));
}

/** Реплика подходит поиску: подстрока в оригинале или переводе без учёта регистра, запрос не короче двух символов. */
export function lineMatches(query: string, ...texts: (string | null | undefined)[]): boolean {
  const needle = query.trim().toLowerCase();
  if ([...needle].length < 2) return false;
  return texts.some((text) => !!text && text.toLowerCase().includes(needle));
}
