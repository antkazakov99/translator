// @ts-nocheck -- Node runtime tests use native TypeScript loading, outside the browser type environment.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { progressLabel, progressPercent } from '../src/lib/progress.ts';

test('zero means nothing is translated', () => {
  assert.equal(progressPercent(0, 1307), 0);
  assert.equal(progressLabel(0, 1307), '0%');
  assert.equal(progressPercent(0, 0), 0);
  assert.equal(progressLabel(0, 0), '0%');
});

test('a single line of a large file stays visible', () => {
  assert.equal(progressPercent(1, 1307), 1);
  assert.equal(progressLabel(1, 1307), '<1%');
});

test('an unfinished file never reads as complete', () => {
  assert.equal(progressPercent(412, 413), 99);
  assert.equal(progressLabel(412, 413), '99%');
});

test('complete and ordinary files use plain percent', () => {
  assert.equal(progressLabel(413, 413), '100%');
  assert.equal(progressPercent(414, 413), 100);
  assert.equal(progressLabel(1, 4), '25%');
  assert.equal(progressLabel(1, 100), '1%');
});
