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

test('all edges between two scenes become one line with a count', () => {
  const edges = [edge('a', 'b', 'branch', 'x > 1'), edge('a', 'b', 'branch', 'y > 2'), edge('a', 'b', 'jump')];
  const graph = layoutGraph([scene('a'), scene('b')], edges, { focus: 'label:a' });
  assert.equal(graph.links.length, 1);
  const [line] = graph.links;
  assert.equal(line.edges.length, 3);
  assert.equal(line.kind, 'mixed');
  assert.equal(line.label, 'переходов: 3');
  assert.equal(line.title, 'если x > 1\nесли y > 2\nпереход');

  const single = layoutGraph([scene('a'), scene('b')], [edge('a', 'b', 'branch', 'x > 1')], { focus: 'label:a' });
  assert.equal(single.links[0].kind, 'branch');
  assert.equal(single.links[0].label, 'если x > 1');
});

test('labels sit next to the outer scene so rows never overlap', () => {
  const scenes = ['in1', 'in2', 'hub', 'out1', 'out2'].map(name => scene(name));
  const edges = [edge('in1', 'hub', 'choice', 'A'), edge('in2', 'hub', 'choice', 'B'), edge('hub', 'out1', 'choice', 'C'), edge('hub', 'out2', 'choice', 'D')];
  const graph = layoutGraph(scenes, edges, { focus: 'label:hub' });
  const node = key => graph.nodes.find(n => n.key === `label:${key}`);
  for (const link of graph.links) {
    const outer = link.from === 'label:hub' ? node(link.to.replace('label:', '')) : node(link.from.replace('label:', ''));
    assert.ok(link.labelY > outer.y && link.labelY < outer.y + 112, 'подпись в строке своей карточки');
  }
  const outLabels = graph.links.filter(l => l.from === 'label:hub');
  assert.ok(outLabels.every(l => l.labelAnchor === 'end'));
  assert.notEqual(outLabels[0].labelY, outLabels[1].labelY);
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
