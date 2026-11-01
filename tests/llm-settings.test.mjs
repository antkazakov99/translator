// @ts-nocheck -- Node runtime tests use native TypeScript loading, outside the browser type environment.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { connectionFor, newConnection, parseLlmSettings, readyToCheck, targetFor, textModels, withoutConnection } from '../src/lib/llm-settings.ts';

const noLegacy = () => null;

test('fresh settings point every action at the local server', () => {
  const settings = parseLlmSettings(null, noLegacy);
  assert.equal(settings.connections.length, 1);
  assert.equal(settings.connections[0].url, 'http://127.0.0.1:1234/v1');
  assert.equal(settings.connections[0].model, 'mistral-small');
  assert.equal(settings.defaultConnection, 'local');
  for (const action of Object.values(settings.actions)) {
    assert.deepEqual(action, { connection: '', model: '' });
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
  assert.equal(settings.connections[0].url, 'http://10.0.0.5:1234/v1');
  assert.deepEqual(settings.actions.names, { connection: 'external', model: 'big' });
  assert.deepEqual(settings.actions.tokens, { connection: '', model: '' });
  assert.equal(settings.connections[0].model, 'qwen');
  assert.equal(settings.connections[1].model, 'big');
  assert.equal(settings.connections[1].key, 'secret');
});

test('settings with a local and an external server become two connections', () => {
  const saved = JSON.stringify({
    localUrl: 'http://a/v1',
    externalUrl: 'https://b/v1',
    externalKey: 'secret',
    actions: { terms: { server: 'external', model: 'm' }, names: { server: 'local', model: 'n' } }
  });
  const settings = parseLlmSettings(saved, noLegacy);
  assert.deepEqual(settings.connections.map((c) => [c.id, c.provider, c.url, c.key]), [
    ['local', 'local', 'http://a/v1', ''],
    ['external', 'compatible', 'https://b/v1', 'secret']
  ]);
  assert.deepEqual(settings.actions.terms, { connection: 'external', model: 'm' });
  assert.deepEqual(settings.actions.names, { connection: 'local', model: 'n' });
  assert.deepEqual(settings.actions.tokens, { connection: '', model: '' });
});

test('models of actions move to their connections and the actions keep working', () => {
  const saved = JSON.stringify({
    connections: [
      { id: 'local', provider: 'local', url: 'http://a/v1', key: '' },
      { id: 'gemini-1', provider: 'gemini', url: 'https://g', key: 'secret' }
    ],
    actions: { descriptions: { connection: 'local', model: 'mistral' }, names: { connection: 'gemini-1', model: 'flash' } }
  });
  const settings = parseLlmSettings(saved, noLegacy);
  assert.deepEqual(settings.connections.map((c) => c.model), ['mistral', 'flash']);
  assert.equal(settings.defaultConnection, 'local');
  assert.equal(targetFor(settings, 'names').model, 'flash');
  assert.equal(targetFor(settings, 'names').provider, 'gemini');
  assert.equal(targetFor(settings, 'tokens').model, 'mistral');
});

test('saved connections survive a round trip', () => {
  const settings = parseLlmSettings(null, noLegacy);
  settings.connections.push(newConnection('claude', settings.connections));
  settings.actions.names = { connection: 'claude-1', model: 'claude-sonnet-5-5' };
  assert.deepEqual(parseLlmSettings(JSON.stringify(settings), noLegacy), settings);
});

test('an action of an unknown connection falls back to the first one', () => {
  const saved = JSON.stringify({ connections: [{ id: 'a', provider: 'openai', url: 'u' }], actions: { names: { connection: 'gone', model: 'm' } } });
  assert.equal(parseLlmSettings(saved, noLegacy).actions.names.connection, 'a');
});

test('broken settings fall back to defaults', () => {
  assert.equal(parseLlmSettings('{не JSON', noLegacy).defaultConnection, 'local');
});

test('a new connection gets the service address and a free id', () => {
  const first = newConnection('gemini', []);
  assert.equal(first.url, 'https://generativelanguage.googleapis.com/v1beta/openai');
  const second = newConnection('gemini', [first]);
  assert.equal(second.id, 'gemini-2');
  assert.equal(second.name, 'Gemini 2');
});

test('removing a connection moves its actions and clears the model', () => {
  const settings = parseLlmSettings(null, noLegacy);
  settings.connections.push(newConnection('openai', settings.connections));
  settings.actions.names = { connection: 'openai-1', model: 'gpt' };
  const left = withoutConnection(settings, 'openai-1');
  assert.equal(left.connections.length, 1);
  assert.deepEqual(left.actions.names, { connection: '', model: '' });
  assert.equal(left.defaultConnection, 'local');

  settings.defaultConnection = 'openai-1';
  assert.equal(withoutConnection(settings, 'openai-1').defaultConnection, 'local');
});

test('the model list keeps text models', () => {
  const hidden = [
    'text-embedding-3-large', 'nomic-embed-text', 'tts-1', 'gpt-4o-mini-tts', 'gemini-2.5-flash-preview-tts', 'whisper-1',
    'gpt-4o-transcribe', 'dall-e-3', 'gpt-image-1', 'imagen-4.0', 'sora-2', 'veo-3.0', 'omni-moderation-latest',
    'gpt-4o-realtime-preview', 'gpt-4o-audio-preview', 'lyria-3', 'nano-banana-pro', 'gemini-robotics-er',
    'gpt-3.5-turbo-instruct', 'babbage-002', 'davinci-002'
  ];
  const kept = ['claude-sonnet-5-5', 'gemini-2.5-flash', 'gpt-5-mini', 'mistral-7b-instruct-v0.1', 'mistralai/mistral-small-3.2'];
  assert.deepEqual(textModels([...hidden, ...kept]), kept);
});

test('model names sort by number and ignore case', () => {
  assert.deepEqual(textModels(['gpt-10', 'Zeta', 'gpt-4.1', 'alpha']), ['alpha', 'gpt-4.1', 'gpt-10', 'Zeta']);
});

test('models are requested only when the connection can answer', () => {
  const claude = newConnection('claude', []);
  assert.equal(readyToCheck(claude), false);
  assert.equal(readyToCheck({ ...claude, key: 'secret' }), true);
  assert.equal(readyToCheck({ ...claude, key: 'secret', url: ' ' }), false);
  assert.equal(readyToCheck(newConnection('local', [])), true);
  assert.equal(readyToCheck({ ...newConnection('compatible', []), url: 'http://10.0.0.5:1234/v1' }), true);
});

test('the target carries the key and the kind of service', () => {
  const settings = parseLlmSettings(null, noLegacy);
  settings.connections.push({ ...newConnection('claude', settings.connections), url: ' https://example.test ', key: ' secret ' });
  settings.actions.names = { connection: 'claude-1', model: ' big ' };

  assert.deepEqual(targetFor(settings, 'names'), { apiUrl: 'https://example.test', model: 'big', apiKey: 'secret', provider: 'claude' });
  assert.deepEqual(targetFor(settings, 'tokens'), { apiUrl: 'http://127.0.0.1:1234/v1', model: 'mistral-small', apiKey: null, provider: 'compatible' });
});

test('an action takes the default until it picks its own', () => {
  const settings = parseLlmSettings(null, noLegacy);
  settings.connections.push({ ...newConnection('gemini', settings.connections), model: 'flash' });
  assert.equal(targetFor(settings, 'names').model, 'mistral-small');

  settings.defaultConnection = 'gemini-1';
  assert.equal(connectionFor(settings, 'names').id, 'gemini-1');
  assert.equal(targetFor(settings, 'names').model, 'flash');

  settings.actions.names = { connection: 'local', model: '' };
  assert.equal(targetFor(settings, 'names').model, 'mistral-small');

  settings.actions.names = { connection: 'gemini-1', model: ' pro ' };
  assert.equal(targetFor(settings, 'names').model, 'pro');
  assert.equal(targetFor(settings, 'tokens').model, 'flash');
});
