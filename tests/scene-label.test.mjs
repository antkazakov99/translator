// @ts-nocheck -- Node runtime tests use native TypeScript loading, outside the browser type environment.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { sceneLabel } from '../src/lib/scene-label.ts';

test('scene title goes with its label, and the label stays alone without a title', () => {
  assert.equal(sceneLabel('day1', ' Ссора в баре '), 'Ссора в баре (day1)');
  assert.equal(sceneLabel('day1', '  '), 'day1');
  assert.equal(sceneLabel('day1'), 'day1');
  assert.equal(sceneLabel(''), '');
});
