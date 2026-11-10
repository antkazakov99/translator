// @ts-nocheck -- Node runtime tests use native TypeScript loading, outside the browser type environment.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { normalizeReadingDepth, partsFor } from '../src/lib/reading-depth.ts';

test('depth normalizes invalid input and inconsistent bounds', () => {
  assert.deepEqual(normalizeReadingDepth({ min: NaN, max: Infinity, percent: NaN }), { min: 1, max: null, percent: 100 }, "не число в максимуме: без предела");
  assert.deepEqual(normalizeReadingDepth({ min: 5.8, max: 2, percent: 200 }), { min: 5, max: 5, percent: 100 });
  assert.deepEqual(normalizeReadingDepth({ min: -2, max: 4.9, percent: -10 }), { min: 1, max: 4, percent: 1 });
});

test('percent is a whole number: Rust keeps it as u8', () => {
  assert.equal(normalizeReadingDepth({ min: 1, max: 5, percent: 33.6 }).percent, 34);
});

test("parts follow the Rust formula: percent rounded up, kept within min and max", () => {
  const depth = { min: 5, max: 50, percent: 30 };
  assert.equal(partsFor(depth, 3), 3, "меньше минимума: все части");
  assert.equal(partsFor(depth, 5), 5);
  assert.equal(partsFor(depth, 10), 5, "3 поднимается до минимума");
  assert.equal(partsFor(depth, 21), 7, "6,3 округляется вверх");
  assert.equal(partsFor(depth, 1000), 50, "не больше максимума");
  assert.equal(partsFor(depth, 0), 0);
});

test("without a max only the percent and the min limit the parts", () => {
  const depth = { min: 5, max: null, percent: 30 };
  assert.deepEqual(normalizeReadingDepth(depth), depth);
  assert.equal(partsFor(depth, 1000), 300);
  assert.equal(partsFor(depth, 10), 5);
});
