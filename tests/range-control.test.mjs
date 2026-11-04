import { test } from 'node:test';
import assert from 'node:assert/strict';
import { syncRange } from '../src/lib/range-control.ts';

// Минимальная модель range: изменение границ браузером ограничивает текущую позицию.
function rangeInput() {
  let min = 0;
  let max = 100;
  let value = 50;
  const clamp = () => { value = Math.min(max, Math.max(min, value)); };
  return {
    get value() { return String(value); },
    set value(next) { value = Number(next); clamp(); },
    get max() { return String(max); },
    set max(next) { max = Number(next); clamp(); },
    get min() { return String(min); },
    set min(next) { min = Number(next); clamp(); },
    step: '1'
  };
}

test('the thumb reaches the end when a 19-part plan replaces a smaller range', () => {
  const input = rangeInput();
  const control = syncRange(input, { max: 3, value: 19 });
  assert.equal(input.value, '3');
  // Значение плана не менялось, но предел вырос после подсчёта частей.
  control.update({ max: 19, value: 19 });
  assert.equal(input.value, '19');
  control.update({ max: 1, value: 19 });
  control.update({ max: 19, value: 19 });
  assert.equal(input.value, '19');
});

test('the range stays within bounds and restores values when switching entries', () => {
  const input = rangeInput();
  const control = syncRange(input, { max: 50, value: 25 });
  assert.equal(input.value, '25');
  control.update({ max: 5, value: 5 });
  assert.equal(input.value, '5');
  control.update({ max: 50, value: 25 });
  assert.equal(input.value, '25');
  control.update({ max: 0, value: 0 });
  assert.equal(input.value, '0');
});
