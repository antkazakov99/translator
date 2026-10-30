// @ts-nocheck -- Node runtime tests use native TypeScript loading, outside the browser type environment.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { buildSceneGraph } from '../src/lib/scene-graph.ts';

const scene = (name) => ({ id: name, name, total_lines: 0, translated_lines: 0, entries: [], choices: [], jumps: [] });
test('branches use actual edges and keep unrelated scenes disconnected', () => {
  const graph = buildSceneGraph(['start', 'left', 'right', 'unrelated'].map(scene), [
    { from: 'start', to: 'left', kind: 'choice', condition: 'Yes' },
    { from: 'start', to: 'right', kind: 'choice', condition: 'No' }
  ]);
  assert.equal(graph.links.length, 2);
  const [start, left, right] = graph.nodes;
  assert.ok(left.x > start.x);
  assert.equal(left.x, right.x);
  assert.notEqual(left.y, right.y);
  assert.ok(!graph.links.some(e => e.to === 'unrelated'));
});
test('cycles, self loops, external labels and similar names remain distinct', () => {
  const graph = buildSceneGraph(['end', 'end_scene'].map(scene), [
    { from: 'end', to: 'end_scene', kind: 'jump', condition: null },
    { from: 'end_scene', to: 'end', kind: 'jump', condition: null },
    { from: 'end', to: 'end', kind: 'jump', condition: null },
    { from: 'end', to: 'other_file', kind: 'call', condition: null }
  ]);
  assert.equal(graph.nodes.length, 3);
  assert.equal(graph.nodes.find(n => n.name === 'other_file')?.scene, undefined);
  assert.equal(graph.links.length, 4);
  assert.ok(graph.links.every(e => !e.path.includes('NaN')));
  assert.equal(new Set(graph.nodes.map(n => `${n.x},${n.y}`)).size, 3);
});
test('choice and jump metadata supply missing links without duplicate choices', () => {
  const start = scene('start');
  start.choices = [{ prompt: null, options: [{ text: 'Go', target_jump: 'next' }, { text: 'Wait', target_jump: null }] }];
  start.jumps = ['next', 'outside'];
  const graph = buildSceneGraph([start, scene('next')], [{ from: 'start', to: 'next', kind: 'choice', condition: 'Go' }]);
  assert.equal(graph.links.length, 2);
  assert.equal(graph.nodes.length, 3);
});
test('empty graph has usable dimensions', () => {
  const graph = buildSceneGraph([], []);
  assert.equal(graph.nodes.length, 0);
  assert.ok(graph.width > 0 && graph.height > 0);
});

