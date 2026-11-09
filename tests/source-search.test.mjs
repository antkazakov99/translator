// @ts-nocheck -- Node runtime tests use native TypeScript loading, outside the browser type environment.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { matchingLines } from '../src/lib/source-search.ts';

test('search ignores case and surrounding spaces', () => {
  assert.deepEqual(matchingLines(['label start:', '    e "Привет."', '    E "привет!"'], '  ПРИВЕТ '), [1, 2]);
});

test('an empty query finds nothing', () => {
  assert.deepEqual(matchingLines(['a', 'b'], '   '), []);
});
