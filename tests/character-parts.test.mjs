// @ts-nocheck -- Node runtime tests use native TypeScript loading, outside the browser type environment.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { partsToRead } from '../src/lib/character-parts.ts';

test('the slider value is the number of parts', () => {
  assert.equal(partsToRead({ total: 5, ready: 0 }, 1), 1);
  assert.equal(partsToRead({ total: 5, ready: 0 }, 3), 3);
});

test('ready parts always stay in the run', () => {
  assert.equal(partsToRead({ total: 9, ready: 3 }, 1), 3);
  assert.equal(partsToRead({ total: 9, ready: 3 }, 4), 4);
});

test('a small role reads everything and stops', () => {
  assert.equal(partsToRead({ total: 2, ready: 0 }, 301), 2);
  assert.equal(partsToRead({ total: 1, ready: 1 }, 301), 1);
});
