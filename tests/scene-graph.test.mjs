// @ts-nocheck -- Node runtime tests use native TypeScript loading, outside the browser type environment.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { edgeLabel, layoutGraph, sceneName } from '../src/lib/scene-graph.ts';

const scene = (name, title = null) => ({
  key: `label:${name}`,
  title,
  file: null,
  total: 0,
  translated: 0,
  drafts: 0,
  firstLine: null,
});
const edge = (from, to, kind = 'jump', condition = '') => ({ from: `label:${from}`, to: `label:${to}`, kind, condition });
const names = (graph, role) => graph.nodes.filter(n => n.role === role).map(n => n.key.replace('label:', ''));

test('empty graph has usable dimensions', () => {
  const graph = layoutGraph([], []);
  assert.equal(graph.nodes.length, 0);
  assert.equal(graph.center, null);
  assert.ok(graph.width > 0 && graph.height > 0);
});

test('only the focused scene and its neighbours are shown', () => {
  const scenes = ['prev', 'current', 'next1', 'next2', 'unrelated1', 'unrelated2'].map(name => scene(name));
  const edges = [
    edge('prev', 'current'),
    edge('current', 'next1', 'choice', 'Option A'),
    edge('current', 'next2', 'choice', 'Option B'),
    edge('unrelated1', 'unrelated2'),
  ];
  const graph = layoutGraph(scenes, edges, { focus: 'label:current' });

  assert.deepEqual(names(graph, 'incoming'), ['prev']);
  assert.deepEqual(names(graph, 'center'), ['current']);
  assert.deepEqual(names(graph, 'outgoing'), ['next1', 'next2']);
  const x = name => graph.nodes.find(n => n.key === `label:${name}`).x;
  assert.ok(x('prev') < x('current') && x('current') < x('next1'));
  assert.equal(x('next1'), x('next2'));
  assert.equal(graph.links.length, 3);
  assert.ok(graph.links.every(l => l.from === 'label:current' || l.to === 'label:current'));
});

test('without a focus the first scene is the centre', () => {
  const graph = layoutGraph([scene('start'), scene('end')], [edge('start', 'end')]);
  assert.equal(graph.center, 'label:start');
  assert.deepEqual(names(graph, 'outgoing'), ['end']);
});

test('several edges between the same scenes keep their own lines', () => {
  const graph = layoutGraph([scene('a'), scene('b')], [edge('a', 'b', 'choice', 'Yes'), edge('a', 'b', 'jump')], { focus: 'label:a' });
  assert.equal(graph.nodes.filter(n => n.role === 'outgoing').length, 1);
  assert.equal(graph.links.length, 2);
  assert.notEqual(graph.links[0].path, graph.links[1].path);
});

test('self loops are drawn above the centre', () => {
  const graph = layoutGraph([scene('loop')], [edge('loop', 'loop')], { focus: 'label:loop' });
  assert.equal(graph.nodes.length, 1);
  assert.equal(graph.links.length, 1);
  assert.ok(graph.links[0].labelY < graph.nodes[0].y);
});

test('columns search by label and by title', () => {
  const scenes = [scene('start'), scene('left'), scene('right', 'Ссора в баре'), scene('in_a', 'Встреча'), scene('in_b')];
  const edges = [edge('start', 'left'), edge('start', 'right'), edge('in_a', 'start'), edge('in_b', 'start')];
  const focus = 'label:start';
  assert.deepEqual(names(layoutGraph(scenes, edges, { focus, outgoingFilter: 'ссора' }), 'outgoing'), ['right']);
  assert.deepEqual(names(layoutGraph(scenes, edges, { focus, outgoingFilter: 'left' }), 'outgoing'), ['left']);
  assert.deepEqual(names(layoutGraph(scenes, edges, { focus, incomingFilter: 'встреча' }), 'incoming'), ['in_a']);
});

test('large columns are paged', () => {
  const into = Array.from({ length: 45 }, (_, i) => `in_${i}`);
  const out = Array.from({ length: 25 }, (_, i) => `out_${i}`);
  const scenes = ['hub', ...into, ...out].map(name => scene(name));
  const edges = [...into.map(name => edge(name, 'hub')), ...out.map(name => edge('hub', name))];

  const first = layoutGraph(scenes, edges, { focus: 'label:hub', pageSize: 10 });
  assert.deepEqual(first.incoming, { total: 45, found: 45, pages: 5, page: 1 });
  assert.deepEqual(first.outgoing, { total: 25, found: 25, pages: 3, page: 1 });
  assert.equal(first.nodes.length, 21);
  assert.equal(first.links.length, 20);

  const second = layoutGraph(scenes, edges, { focus: 'label:hub', pageSize: 10, incomingPage: 2 });
  assert.equal(names(second, 'incoming')[0], 'in_10');

  const beyond = layoutGraph(scenes, edges, { focus: 'label:hub', pageSize: 10, outgoingPage: 99 });
  assert.equal(beyond.outgoing.page, 3);

  const filtered = layoutGraph(scenes, edges, { focus: 'label:hub', incomingFilter: 'in_4' });
  assert.equal(filtered.incoming.found, 6);
  assert.equal(filtered.incoming.total, 45);
});

test('a hub with a thousand entries stays compact and fast', () => {
  const edges = Array.from({ length: 1060 }, (_, i) => edge(`from_${i}`, 'house'));
  const started = performance.now();
  const graph = layoutGraph([scene('house')], edges, { focus: 'label:house', pageSize: 10 });
  const elapsed = performance.now() - started;
  assert.equal(graph.incoming.pages, 106);
  assert.equal(graph.nodes.length, 11);
  assert.ok(graph.height < 3000);
  assert.ok(elapsed < 100, `${elapsed}ms`);
});

test('scene names and edge labels read well', () => {
  assert.equal(sceneName('label:day1'), 'day1');
  assert.equal(sceneName('label:day1', scene('day1', 'Пляж')), 'Пляж · day1');
  assert.equal(edgeLabel({ from: 'a', to: 'b', kind: 'choice', condition: 'Выбор: "Yes"' }), '"Yes"');
  assert.equal(edgeLabel({ from: 'a', to: 'b', kind: 'branch', condition: 'met' }), 'если met');
  assert.equal(edgeLabel({ from: 'a', to: 'b', kind: 'screen', condition: 'map' }), 'экран map');
  assert.equal(edgeLabel({ from: 'a', to: 'b', kind: 'jump', condition: '' }), '');
});
