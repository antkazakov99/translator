// @ts-nocheck -- Node runtime tests use native TypeScript loading, outside the browser type environment.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { distributeRequests, requestRange, requestsFor } from '../src/lib/character-requests.ts';

test('a single new part is read by the request that writes the description', () => {
  assert.equal(requestsFor({ total: 5, ready: 0 }, 1), 1);
  assert.equal(requestsFor({ total: 5, ready: 0 }, 2), 3);
});

test('ready parts cost nothing but the description is still written', () => {
  assert.equal(requestsFor({ total: 5, ready: 2 }, 2), 1);
  assert.equal(requestsFor({ total: 5, ready: 2 }, 5), 4);
  assert.equal(requestsFor({ total: 1, ready: 1 }, 1), 1);
});

test('the range runs from one part each to every part', () => {
  assert.deepEqual(requestRange([{ total: 5, ready: 0 }, { total: 2, ready: 2 }]), { min: 2, max: 7 });
  assert.deepEqual(requestRange([]), { min: 0, max: 0 });
});

test('the lowest budget reads one part and keeps ready parts', () => {
  assert.deepEqual(distributeRequests([{ total: 5, ready: 0 }, { total: 9, ready: 3 }], 2), [1, 3]);
});

test('the highest budget reads everything', () => {
  const characters = [{ total: 5, ready: 0 }, { total: 9, ready: 3 }, { total: 1, ready: 0 }];
  assert.deepEqual(distributeRequests(characters, requestRange(characters).max), [5, 9, 1]);
});

test('larger roles get more parts', () => {
  const characters = [{ total: 40, ready: 0 }, { total: 4, ready: 0 }];
  const [big, small] = distributeRequests(characters, 24);
  assert.ok(big > small * 3, `${big} и ${small}`);
});

test('the plan never exceeds the budget', () => {
  const characters = [{ total: 6, ready: 0 }, { total: 3, ready: 1 }, { total: 2, ready: 0 }];
  const { min, max } = requestRange(characters);
  for (let budget = min; budget <= max; budget += 1) {
    const parts = distributeRequests(characters, budget);
    const used = characters.reduce((sum, c, i) => sum + requestsFor(c, parts[i]), 0);
    assert.ok(used <= budget, `${used} из ${budget}`);
    assert.ok(budget - used <= 1, `осталось ${budget - used} из ${budget}`);
  }
});
