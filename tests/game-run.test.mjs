// @ts-nocheck -- Node runtime tests use native TypeScript loading, outside the browser type environment.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { availableSteps, chainText, defaultSteps, fedBy, finishedText, leftLabel, plural, requestsLabel, stepOutcomes, stepsToRun } from '../src/lib/game-run.ts';

const work = (step, counts, requests = 0) => ({
  step,
  title: step,
  left: counts.map(([label, count]) => ({ label, count })),
  requests,
  empty: counts.every(([, count]) => count === 0),
});

test('russian plural forms of requests', () => {
  assert.equal(plural(1, ['запрос', 'запроса', 'запросов']), 'запрос');
  assert.equal(plural(3, ['запрос', 'запроса', 'запросов']), 'запроса');
  assert.equal(plural(11, ['запрос', 'запроса', 'запросов']), 'запросов');
  assert.equal(plural(21, ['запрос', 'запроса', 'запросов']), 'запрос');
  assert.equal(requestsLabel(12300).replace(/\s/g, ' '), '≈ 12 300 запросов');
});

test('left work shows only what is there', () => {
  assert.equal(leftLabel(work('glossary', [['терминов без перевода', 5], ['подписей без перевода', 0]])), 'терминов без перевода: 5');
  assert.equal(leftLabel(work('lines', [['реплик без перевода', 0]])), 'Делать нечего');
});

test('steps without work are neither default nor run', () => {
  const all = [work('review', [['реплик не вычитано', 3]]), work('analysis', [['сцен', 2]]), work('lines', [['реплик', 0]])];
  assert.deepEqual(defaultSteps(all), ['analysis', 'review']);
  assert.deepEqual(stepsToRun(['review', 'lines', 'analysis'], all), ['analysis', 'review'], 'порядок цепочки, пустой шаг выпал');
});

test('a step gets work from a chosen earlier step', () => {
  const all = [
    work('analysis', [['сцен', 140]]),
    work('glossary', [['терминов', 0]]),
    work('entries', [['без описания', 0]]),
    work('scenes', [['сцен без описания', 140]]),
    work('lines', [['реплик', 12300]]),
    work('review', [['не вычитано', 0]]),
  ];
  assert.deepEqual(defaultSteps(all), ['analysis', 'glossary', 'entries', 'scenes', 'lines', 'review'], 'разбор найдёт записи, перевод даст вычитку');
  assert.equal(fedBy('entries', defaultSteps(all), all), 'analysis');
  assert.equal(fedBy('review', defaultSteps(all), all), 'lines');
  assert.equal(fedBy('lines', defaultSteps(all), all), null, 'у перевода работа есть сейчас');

  const withoutAnalysis = ['glossary', 'entries', 'scenes', 'lines', 'review'];
  assert.deepEqual([...availableSteps(withoutAnalysis, all)], ['analysis', 'scenes', 'lines', 'review'], 'без разбора описывать некого, сам разбор отметить можно');
  assert.deepEqual(stepsToRun(withoutAnalysis, all), ['scenes', 'lines', 'review']);
  assert.deepEqual(stepsToRun(['review'], all), [], 'вычитке без перевода нечего читать');
});

test('run bar shows the step of the chain', () => {
  const stage = { index: 2, count: 4, title: 'Перевод терминов' };
  assert.equal(chainText({ done: 0, total: 0, written: 0, message: null, stage }, false), 'Шаг 2 из 4 · Перевод терминов: подготовка');
  assert.equal(
    chainText({ done: 2, total: 5, written: 1, message: 'в пакете: Rusty Anchor', stage }, false),
    'Шаг 2 из 4 · Перевод терминов: запрос 3 из 5 · записано: 1 · в пакете: Rusty Anchor',
  );
  assert.equal(chainText({ done: 2, total: 5, written: 1, message: null }, true), 'Вся игра: останавливается после текущего запроса');
});

test('outcomes mark done, broken, waiting and skipped steps', () => {
  const outcomes = stepOutcomes(['analysis', 'glossary', 'lines'], {
    finished: [{ step: 'analysis', written: 3 }],
    stopped: false,
    error: 'Переводы глоссария: нет связи',
  });
  assert.deepEqual(outcomes.analysis, { kind: 'done', written: 3 });
  assert.deepEqual(outcomes.glossary, { kind: 'error', message: 'Переводы глоссария: нет связи' });
  assert.deepEqual(outcomes.lines, { kind: 'waiting' });
  assert.deepEqual(outcomes.review, { kind: 'skipped' });

  const stopped = stepOutcomes(['lines', 'review'], { finished: [], stopped: true, error: null });
  assert.deepEqual([stopped.lines.kind, stopped.review.kind], ['stopped', 'waiting']);
});

test('finished summary names the steps', () => {
  assert.equal(finishedText([{ step: 'lines', written: 120 }], false), 'Готово: Перевод реплик · 120');
  assert.equal(finishedText([], true), 'Остановлено до конца первого шага');
});
