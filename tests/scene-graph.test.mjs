// @ts-nocheck -- Node runtime tests use native TypeScript loading, outside the browser type environment.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { buildSceneGraph, buildFocusedSceneGraph } from '../src/lib/scene-graph.ts';

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

test('focused mode: displays ONLY center scene, incoming and outgoing nodes', () => {
  // 5 scenes, only prev -> current -> next1, next2. Unrelated scenes should NOT be rendered!
  const scenes = ['prev', 'current', 'next1', 'next2', 'unrelated1', 'unrelated2'].map(scene);
  const edges = [
    { from: 'prev', to: 'current', kind: 'jump', condition: null },
    { from: 'current', to: 'next1', kind: 'choice', condition: 'Option A' },
    { from: 'current', to: 'next2', kind: 'choice', condition: 'Option B' },
    { from: 'unrelated1', to: 'unrelated2', kind: 'jump', condition: null },
  ];

  const graph = buildSceneGraph(scenes, edges, { mode: 'focused', focusName: 'current' });

  // 1 incoming ('prev') + 1 center ('current') + 2 outgoing ('next1', 'next2') = 4 nodes total
  assert.equal(graph.nodes.length, 4);
  assert.equal(graph.incomingCount, 1);
  assert.equal(graph.outgoingCount, 2);

  const prevNode = graph.nodes.find(n => n.name === 'prev');
  const currentNode = graph.nodes.find(n => n.name === 'current');
  const next1Node = graph.nodes.find(n => n.name === 'next1');
  const next2Node = graph.nodes.find(n => n.name === 'next2');

  assert.ok(prevNode, 'Incoming node exists');
  assert.ok(currentNode, 'Center node exists');
  assert.ok(next1Node, 'Outgoing node 1 exists');
  assert.ok(next2Node, 'Outgoing node 2 exists');

  // Horizontal positions: Left < Center < Right
  assert.ok(prevNode.x < currentNode.x);
  assert.ok(currentNode.x < next1Node.x);
  assert.equal(next1Node.x, next2Node.x);

  // Unrelated scenes are NOT in the graph
  assert.ok(!graph.nodes.some(n => n.name.startsWith('unrelated')));

  // Links: only 3 links connected to center
  assert.equal(graph.links.length, 3);
  assert.ok(graph.links.every(l => l.from === 'current' || l.to === 'current'));
});

test('focused mode: handles external jump targets cleanly', () => {
  const scenes = ['chapter1'].map(scene);
  const edges = [
    { from: 'chapter1', to: 'other_script_label', kind: 'jump', condition: null }
  ];

  const graph = buildSceneGraph(scenes, edges, { mode: 'focused', focusName: 'chapter1' });
  assert.equal(graph.nodes.length, 2);
  const external = graph.nodes.find(n => n.name === 'other_script_label');
  assert.ok(external);
  assert.equal(external.scene, undefined);
  assert.equal(external.role, 'outgoing');
});

test('focused mode: handles external incoming nodes from other files cleanly', () => {
  const scenes = ['chapter2_start'].map(scene);
  const edges = [
    { from: 'prologue_end', to: 'chapter2_start', kind: 'jump', condition: null },
    { from: 'chapter2_start', to: 'chapter2_end', kind: 'jump', condition: null }
  ];

  const graph = buildSceneGraph(scenes, edges, { mode: 'focused', focusName: 'chapter2_start' });
  assert.equal(graph.nodes.length, 3);
  assert.equal(graph.incomingCount, 1);
  assert.equal(graph.outgoingCount, 1);

  const incomingExtNode = graph.nodes.find(n => n.name === 'prologue_end');
  assert.ok(incomingExtNode, 'External incoming node must be present');
  assert.equal(incomingExtNode.scene, undefined, 'External node has no local scene');
  assert.equal(incomingExtNode.role, 'incoming');
  assert.ok(incomingExtNode.x < graph.nodes.find(n => n.name === 'chapter2_start').x);
});

test('focused mode: paginates large incoming and outgoing node lists cleanly', () => {
  const current = scene('hub_center');
  const incomingScenes = Array.from({ length: 45 }, (_, i) => scene(`in_scene_${i}`));
  const outgoingScenes = Array.from({ length: 25 }, (_, i) => scene(`out_scene_${i}`));

  const allScenes = [current, ...incomingScenes, ...outgoingScenes];
  const edges = [
    ...incomingScenes.map(s => ({ from: s.name, to: 'hub_center', kind: 'jump', condition: null })),
    ...outgoingScenes.map(s => ({ from: 'hub_center', to: s.name, kind: 'jump', condition: null })),
  ];

  // Page 1: 10 incoming + 1 center + 10 outgoing = 21 nodes
  const page1 = buildFocusedSceneGraph(allScenes, edges, {
    focusName: 'hub_center',
    incomingPage: 1,
    incomingPageSize: 10,
    outgoingPage: 1,
    outgoingPageSize: 10,
  });

  assert.equal(page1.incomingTotalRaw, 45);
  assert.equal(page1.incomingTotalPages, 5);
  assert.equal(page1.incomingCurrentPage, 1);
  assert.equal(page1.outgoingTotalRaw, 25);
  assert.equal(page1.outgoingTotalPages, 3);
  assert.equal(page1.outgoingCurrentPage, 1);

  // Exactly 10 incoming + 1 center + 10 outgoing nodes rendered!
  assert.equal(page1.nodes.filter(n => n.role === 'incoming').length, 10);
  assert.equal(page1.nodes.filter(n => n.role === 'outgoing').length, 10);
  assert.equal(page1.nodes.filter(n => n.role === 'center').length, 1);
  assert.equal(page1.links.length, 20);

  // Page 2: next 10 incoming
  const page2 = buildFocusedSceneGraph(allScenes, edges, {
    focusName: 'hub_center',
    incomingPage: 2,
    incomingPageSize: 10,
    outgoingPage: 1,
    outgoingPageSize: 10,
  });
  assert.equal(page2.incomingCurrentPage, 2);
  const firstInPage2 = page2.nodes.find(n => n.role === 'incoming');
  assert.equal(firstInPage2?.name, 'in_scene_10');

  // Filtering: search for specific incoming node
  const filtered = buildFocusedSceneGraph(allScenes, edges, {
    focusName: 'hub_center',
    incomingFilter: '_scene_4',
  });
  // 'in_scene_4', 'in_scene_40'..'in_scene_44' = 6 matches
  assert.equal(filtered.incomingCount, 6);
  assert.ok(filtered.nodes.filter(n => n.role === 'incoming').every(n => n.name.includes('_scene_4')));
});

test('focused mode: handles 1000+ routes in < 15ms without canvas explosion', () => {
  const current = scene('house');
  const edges = Array.from({ length: 1060 }, (_, i) => ({
    from: `scene_from_${i}`,
    to: 'house',
    kind: 'jump',
    condition: null,
  }));

  const start = performance.now();
  const graph = buildFocusedSceneGraph([current], edges, {
    focusName: 'house',
    incomingPage: 1,
    incomingPageSize: 10,
  });
  const elapsed = performance.now() - start;

  assert.equal(graph.incomingTotalRaw, 1060);
  assert.equal(graph.incomingTotalPages, 106);
  assert.equal(graph.nodes.length, 11); // 10 incoming + 1 center
  assert.equal(graph.links.length, 10);
  assert.ok(graph.height < 3000, 'Height must be compact and not 150,000px!');
  assert.ok(elapsed < 100, `Execution took ${elapsed}ms, should be fast`);
});
