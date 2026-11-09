// @ts-nocheck -- Node runtime tests use native TypeScript loading, outside the browser type environment.
import { test } from "node:test";
import assert from "node:assert/strict";
import { filterScenes, hitMap, lineMatches, pickId, sceneItemName, sceneList } from "../src/lib/scene-tree.ts";

const scene = (key, title = null) => ({ key, title, described: false, stale: false, total: 2, translated: 1, drafts: 0 });
const file = (id, path, scenes) => ({ file: { id, path, total: 10, translated: 5, drafts: 0 }, scenes });

const tree = [
  file(1, "game/script.rpy", [scene("label:start", "Пробуждение"), scene("label:left"), scene(null)]),
  file(2, "game/day2.rpy", [scene("label:beach", "Пляж"), scene("label:left_beach"), scene(null)]),
];

const names = (list) => list.map((entry) => entry.name);

test("names show title and label, loose lines name their file", () => {
  assert.equal(sceneItemName(scene("label:start", "Пробуждение")), "Пробуждение · start");
  assert.equal(sceneItemName(scene("label:left")), "left");
  assert.equal(sceneItemName(scene(null)), "Без сцены");
  assert.equal(sceneItemName(scene(null), "game/script.rpy"), "Без сцены · script.rpy");
});

test("pick ids tell a scene from loose lines of a file", () => {
  assert.equal(pickId({ kind: "scene", key: "label:start" }), "scene:label:start");
  assert.equal(pickId({ kind: "loose", file: 3 }), "loose:3");
  assert.notEqual(pickId({ kind: "loose", file: 3 }), pickId({ kind: "loose", file: 4 }));
});

test("scenes go by their original name, loose lines last by file", () => {
  const list = sceneList(tree);
  assert.deepEqual(names(list), ["Пляж · beach", "left", "left_beach", "Пробуждение · start", "Без сцены · day2.rpy", "Без сцены · script.rpy"]);
  assert.deepEqual(list[4].pick, { kind: "loose", file: 2 });
});

test("the translated title comes first and the original under it", () => {
  const [beach, left] = sceneList(tree);
  assert.deepEqual([beach.title, beach.original], ["Пляж", "beach"]);
  assert.deepEqual([left.title, left.original], [null, "left"], "без перевода названия нет, оригинал на месте");
  const loose = sceneList(tree).at(-1);
  assert.deepEqual([loose.title, loose.original], ["Без сцены", "script.rpy"]);
});

test("numbers in names compare as numbers and case is ignored", () => {
  const list = sceneList([file(1, "game/a.rpy", [scene("label:day10"), scene("label:Day2"), scene("label:day1")])]);
  assert.deepEqual(names(list), ["day1", "Day2", "day10"]);
});

test("without a query everything stays", () => {
  const list = sceneList(tree);
  assert.equal(filterScenes(list, "  ").length, list.length);
});

test("query matches titles, labels and loose lines", () => {
  const list = sceneList(tree);
  assert.deepEqual(names(filterScenes(list, "пляж")), ["Пляж · beach"]);
  assert.deepEqual(names(filterScenes(list, "LEFT")), ["left", "left_beach"]);
  assert.deepEqual(names(filterScenes(list, "без сцены")), ["Без сцены · day2.rpy", "Без сцены · script.rpy"]);
  assert.deepEqual(names(filterScenes(list, "label:")), [], "вид ключа не ищется");
});

test("a matching file path keeps the whole file", () => {
  assert.deepEqual(names(filterScenes(sceneList(tree), "day2")), ["Пляж · beach", "left_beach", "Без сцены · day2.rpy"]);
});

test("scenes found by their lines join the ones found by name", () => {
  const list = sceneList(tree);
  const hits = hitMap([
    { key: "label:start", file: 1, lines: 3 },
    { key: null, file: 2, lines: 1 },
  ]);
  assert.equal(hits.get("scene:label:start"), 3);
  assert.deepEqual(names(filterScenes(list, "пляж", hits)), ["Пляж · beach", "Пробуждение · start", "Без сцены · day2.rpy"]);
});

test("a line matches by original or translation, ignoring case", () => {
  assert.ok(lineMatches(" эйлин ", "Hello.", "Привет, Эйлин."));
  assert.ok(lineMatches("HELLO", "Hello.", null));
  assert.ok(!lineMatches("bye", "Hello.", "Привет."));
  assert.ok(!lineMatches("h", "Hello."), "один символ не ищется");
});
