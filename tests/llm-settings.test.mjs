// @ts-nocheck -- Node runtime tests use native TypeScript loading, outside the browser type environment.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { actionLabel, parseLlmSettings, targetFor } from '../src/lib/llm-settings.ts';

const noLegacy = () => null;

test('fresh settings point every action at the local server', () => {
  const settings = parseLlmSettings(null, noLegacy);
  assert.equal(settings.localUrl, 'http://127.0.0.1:1234/v1');
  for (const action of Object.values(settings.actions)) {
    assert.deepEqual(action, { server: 'local', model: 'mistral-small' });
  }
});

test('older connection settings carry over', () => {
  const legacy = {
    'translator.llm_api_url': 'http://10.0.0.5:1234/v1',
    'translator.llm_model': 'qwen',
    'translator.llm_names_api_url': 'https://example.test/v1',
    'translator.llm_names_model': 'big',
    'translator.llm_names_api_key': 'secret'
  };
  const settings = parseLlmSettings(null, (key) => legacy[key] ?? null);
  assert.equal(settings.localUrl, 'http://10.0.0.5:1234/v1');
  assert.deepEqual(settings.actions.names, { server: 'external', model: 'big' });
  assert.deepEqual(settings.actions.tokens, { server: 'local', model: 'qwen' });
  assert.equal(settings.externalKey, 'secret');
});

test('saved settings win and a new action gets the default', () => {
  const saved = JSON.stringify({ localUrl: 'http://a/v1', actions: { terms: { server: 'external', model: 'm' } } });
  const settings = parseLlmSettings(saved, noLegacy);
  assert.equal(settings.localUrl, 'http://a/v1');
  assert.deepEqual(settings.actions.terms, { server: 'external', model: 'm' });
  assert.deepEqual(settings.actions.tokens, { server: 'local', model: 'mistral-small' });
});

test('broken settings fall back to defaults', () => {
  assert.equal(parseLlmSettings('{не JSON', noLegacy).actions.names.server, 'local');
});

test('the key goes only to the external server', () => {
  const settings = parseLlmSettings(null, noLegacy);
  settings.externalUrl = ' https://example.test/v1 ';
  settings.externalKey = ' secret ';
  settings.actions.names = { server: 'external', model: ' big ' };

  assert.deepEqual(targetFor(settings, 'names'), { apiUrl: 'https://example.test/v1', model: 'big', apiKey: 'secret' });
  assert.equal(targetFor(settings, 'tokens').apiKey, null);
  assert.equal(targetFor(settings, 'tokens').apiUrl, 'http://127.0.0.1:1234/v1');
});

test('the label names the model and the server', () => {
  const settings = parseLlmSettings(null, noLegacy);
  assert.equal(actionLabel(settings, 'names'), 'mistral-small, локальный сервер');
  settings.actions.names = { server: 'external', model: '' };
  assert.equal(actionLabel(settings, 'names'), 'модель не выбрана, внешний сервер');
});
