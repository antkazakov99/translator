import { test } from 'node:test';
import assert from 'node:assert/strict';
import { newProviderName, optionalInteger, optionalTemperature, textModels } from '../src/lib/llm-catalog.ts';

test('a new provider takes a free name', () => {
  assert.equal(newProviderName('claude', []), 'Claude');
  assert.equal(newProviderName('claude', ['Claude', 'Claude 2']), 'Claude 3');
});

test('the model list keeps text models sorted by number', () => {
  assert.deepEqual(textModels(['gpt-10', 'text-embedding-3', 'gpt-4.1', 'whisper-1', 'Gemma']), ['Gemma', 'gpt-4.1', 'gpt-10']);
});

test('empty number fields mean defaults and bad ones are rejected', () => {
  assert.equal(optionalInteger('', 1), null);
  assert.equal(optionalInteger(' 32000 ', 1), 32000);
  assert.equal(optionalInteger('0', 1), undefined);
  assert.equal(optionalInteger('0', 0), 0);
  assert.equal(optionalInteger('1.5', 1), undefined);
  assert.equal(optionalTemperature('0,3'), 0.3);
  assert.equal(optionalTemperature(''), null);
  assert.equal(optionalTemperature('2.1'), undefined);
  assert.equal(optionalTemperature('-1'), undefined);
});
