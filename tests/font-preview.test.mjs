// @ts-nocheck -- Node runtime tests use native TypeScript loading, outside the browser type environment.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fontOrder, fontStatus, needsReplacement, sampleSegments } from '../src/lib/font-preview.ts';

const LETTERS = 'АБВГДЕЁЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯабвгдеёжзийклмнопрстуфхцчшщъыьэюя';
const coverage = (missing_target = '', missing_original = '', unreadable = false) => ({ unreadable, missing_original, missing_target });
const font = (fields) => ({ name: 'a.ttf', origin: 'game', references: 1, coverage: coverage(), locked: null, replacement: null, ...fields });

test('missing characters are marked so a system font does not hide them', () => {
  assert.deepEqual(sampleSegments('Да — нет', 'а—'), [
    { text: 'Д', missing: false },
    { text: 'а', missing: true },
    { text: ' ', missing: false },
    { text: '—', missing: true },
    { text: ' нет', missing: false }
  ]);
  assert.deepEqual(sampleSegments('abc', ''), [{ text: 'abc', missing: false }]);
  assert.deepEqual(sampleSegments('', 'a'), []);
});

test('status separates missing letters, marks and symbol fonts', () => {
  assert.equal(fontStatus(coverage()).kind, 'ok');
  assert.equal(fontStatus(coverage('«»—')).kind, 'marks');
  assert.match(fontStatus(coverage('«»—')).label, /«/);
  const none = fontStatus(coverage(LETTERS + '«»—…'));
  assert.equal(none.kind, 'letters');
  assert.match(none.label, /кириллицы/);
  assert.match(fontStatus(coverage('Ёё')).label, /Ёё/);
  assert.equal(fontStatus(coverage('Ё«»—…')).label, 'Нет букв: Ё; знаков: « » — …');
  assert.equal(fontStatus(coverage(LETTERS, 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz')).kind, 'symbols');
  assert.equal(fontStatus(coverage('', '', true)).kind, 'unknown');
});

test('fonts that need a decision come first and engine fonts last', () => {
  const waiting = font({ name: 'w', coverage: coverage(LETTERS) });
  const engine = font({ name: 'e', origin: 'engine' });
  const fine = font({ name: 'f' });
  assert.deepEqual([engine, fine, waiting].sort(fontOrder).map((item) => item.name), ['w', 'f', 'e']);
});

test('only text fonts without the language and without a replacement need one', () => {
  assert.equal(needsReplacement(font({ coverage: coverage(LETTERS) })), true);
  assert.equal(needsReplacement(font({ coverage: coverage('«') })), true);
  assert.equal(needsReplacement(font()), false);
  assert.equal(needsReplacement(font({ coverage: coverage(LETTERS), origin: 'engine' })), false);
  assert.equal(needsReplacement(font({ coverage: coverage(LETTERS), replacement: { source_name: 'b.ttf', coverage: coverage() } })), false);
  assert.equal(needsReplacement(font({ coverage: coverage(LETTERS, 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz') })), false);
});
