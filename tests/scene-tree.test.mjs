// @ts-nocheck -- Node runtime tests use native TypeScript loading, outside the browser type environment.
import { test } from "node:test";
import assert from "node:assert/strict";
import { filterScenes, pickId, sceneItemName, sceneList } from "../src/lib/scene-tree.ts";

const scene = (key, title = null) => ({ key, title, described: false, stale: false, total: 2, translated: 1, drafts: 0 });
const file = (id, path, scenes) => ({ file: { id, path, total: 10, translated: 5, drafts: 0 }, scenes });

const tree = [
  file(1, "game/script.rpy", [scene("label:start", "Пробуждение"), scene("label:left"), scene(null)]),
  file(2, "game/day2.rpy", [scene("label:beach", "Пляж"), scene("label:left_beach")]),
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

test("the list is flat in file order with loose lines after the scenes of their file", () => {
  const list = sceneList(tree);
  assert.deepEqual(names(list), ["Пробуждение · start", "left", "Без сцены · script.rpy", "Пляж · beach", "left_beach"]);
  assert.deepEqual(list[2].pick, { kind: "loose", file: 1 });
  assert.equal(list[3].file.id, 2);
});

test("without a query everything stays", () => {
  const list = sceneList(tree);
  assert.equal(filterScenes(list, "  ").length, list.length);
});

test("query matches titles, labels and loose lines", () => {
  const list = sceneList(tree);
  assert.deepEqual(names(filterScenes(list, "пляж")), ["Пляж · beach"]);
  assert.deepEqual(names(filterScenes(list, "LEFT")), ["left", "left_beach"]);
  assert.deepEqual(names(filterScenes(list, "без сцены")), ["Без сцены · script.rpy"]);
  assert.deepEqual(names(filterScenes(list, "label:")), [], "вид ключа не ищется");
});

test("a matching file path keeps the whole file", () => {
  assert.deepEqual(names(filterScenes(sceneList(tree), "day2")), ["Пляж · beach", "left_beach"]);
});
