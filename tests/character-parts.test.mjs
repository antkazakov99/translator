// @ts-nocheck -- Node runtime tests use native TypeScript loading, outside the browser type environment.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { DEFAULT_READING_DEPTH, normalizeReadingDepth, partsForDepth, nothingToRead, partsToRead } from '../src/lib/character-parts.ts';

test('depth applies the percentage separately to each entry and rounds up', () => {
  const depth = { min: 2, max: 10, percent: 25 };
  assert.equal(partsForDepth({ total: 20, ready: 0 }, depth), 5);
  assert.equal(partsForDepth({ total: 21, ready: 0 }, depth), 6);
  assert.equal(partsForDepth({ total: 100, ready: 0 }, depth), 10);
  assert.equal(partsForDepth({ total: 4, ready: 0 }, depth), 2);
});

test('depth never requests more parts than exist, including empty entries', () => {
  const depth = { min: 5, max: 10, percent: 50 };
  assert.equal(partsForDepth({ total: 2, ready: 0 }, depth), 2);
  assert.equal(partsForDepth({ total: 0, ready: 0 }, depth), 0);
});

test('ready notes survive a reduced maximum and only missing parts are added', () => {
  const depth = { min: 2, max: 5, percent: 50 };
  assert.equal(partsForDepth({ total: 20, ready: 8 }, depth), 8);
  assert.equal(partsForDepth({ total: 20, ready: 3 }, depth), 5);
});

test('100 percent reads everything within the maximum', () => {
  assert.equal(partsForDepth({ total: 17, ready: 0 }, { min: 1, max: 20, percent: 100 }), 17);
  assert.equal(partsForDepth({ total: 17, ready: 0 }, DEFAULT_READING_DEPTH), 6);
});

test('depth normalizes invalid input and inconsistent bounds', () => {
  assert.deepEqual(normalizeReadingDepth({ min: NaN, max: Infinity, percent: NaN }), { min: 1, max: 1, percent: 100 });
  assert.deepEqual(normalizeReadingDepth({ min: 5.8, max: 2, percent: 200 }), { min: 5, max: 5, percent: 100 });
  assert.deepEqual(normalizeReadingDepth({ min: -2, max: 4.9, percent: -10 }), { min: 1, max: 4, percent: 1 });
});

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

test('a run below the ready parts reads nothing new', () => {
  assert.equal(nothingToRead({ total: 21, ready: 10 }, 5), true);
  assert.equal(nothingToRead({ total: 21, ready: 10 }, 10), true);
  assert.equal(nothingToRead({ total: 21, ready: 10 }, 11), false);
});

test('a fully read role has nothing to read at any slider value', () => {
  assert.equal(nothingToRead({ total: 2, ready: 2 }, 301), true);
  assert.equal(nothingToRead({ total: 2, ready: 1 }, 301), false);
  assert.equal(nothingToRead({ total: 2, ready: 0 }, 1), false);
});
