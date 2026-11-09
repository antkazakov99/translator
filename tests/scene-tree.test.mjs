// @ts-nocheck -- Node runtime tests use native TypeScript loading, outside the browser type environment.
import { test } from "node:test";
import assert from "node:assert/strict";
import { filterTree, pickId, sceneItemName } from "../src/lib/scene-tree.ts";

const scene = (key, title = null) => ({ key, title, described: false, stale: false, total: 2, translated: 1, drafts: 0 });
const file = (id, path, scenes) => ({ file: { id, path, total: 10, translated: 5, drafts: 0 }, scenes });

const tree = [
  file(1, "game/script.rpy", [scene("label:start", "Пробуждение"), scene("label:left"), scene(null)]),
  file(2, "game/day2.rpy", [scene("label:beach", "Пляж"), scene("label:left_beach")]),
];

const keys = (view) => view.groups.map(({ group, scenes }) => [group.file.id, scenes.map((item) => item.key)]);

test("names show title and label, loose lines get their own name", () => {
  assert.equal(sceneItemName(scene("label:start", "Пробуждение")), "Пробуждение · start");
  assert.equal(sceneItemName(scene("label:left")), "left");
  assert.equal(sceneItemName(scene(null)), "Без сцены");
});

test("pick ids tell a scene from loose lines of a file", () => {
  assert.equal(pickId({ kind: "scene", key: "label:start" }), "scene:label:start");
  assert.equal(pickId({ kind: "loose", file: 3 }), "loose:3");
  assert.notEqual(pickId({ kind: "loose", file: 3 }), pickId({ kind: "loose", file: 4 }));
});

test("without a query every group keeps every scene", () => {
  const view = filterTree(tree, "  ");
  assert.deepEqual(keys(view), [
    [1, ["label:start", "label:left", null]],
    [2, ["label:beach", "label:left_beach"]],
  ]);
  assert.equal(view.hidden, 0);
});

test("query matches titles and labels, groups without matches disappear", () => {
  assert.deepEqual(keys(filterTree(tree, "пляж")), [[2, ["label:beach"]]]);
  assert.deepEqual(keys(filterTree(tree, "LEFT")), [
    [1, ["label:left"]],
    [2, ["label:left_beach"]],
  ]);
  assert.deepEqual(keys(filterTree(tree, "без сцены")), [[1, [null]]]);
  assert.deepEqual(keys(filterTree(tree, "label:")), [], "вид ключа не ищется");
});

test("a matching file path shows the whole file", () => {
  assert.deepEqual(keys(filterTree(tree, "day2")), [[2, ["label:beach", "label:left_beach"]]]);
});

test("search shows a limited number of scenes and counts the rest", () => {
  const view = filterTree(tree, "e", 2);
  assert.equal(view.groups.flatMap((group) => group.scenes).length, 2);
  assert.ok(view.hidden > 0);
});
