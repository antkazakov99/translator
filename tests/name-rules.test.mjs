import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile, writeFile, mkdir, unlink } from 'node:fs/promises';
import { randomUUID } from 'node:crypto';
import { compileModule } from 'svelte/compiler';
import ts from 'typescript';

test('rule library shares drafts, preserves unsaved edits, and persists renaming and deletion', async () => {
  // Компилируем настоящий модуль с рунами, подменяя только границу Tauri.
  const source = await readFile(new URL('../src/lib/name-rules.svelte.ts', import.meta.url), 'utf8');
  const input = source.replace('import { invoke } from "@tauri-apps/api/core";', 'const invoke = globalThis.__nameRulesInvoke;');
  assert.notEqual(input, source);
  const javascript = ts.transpileModule(input, { compilerOptions: { target: ts.ScriptTarget.ESNext, module: ts.ModuleKind.ESNext } }).outputText;
  const compiled = compileModule(javascript, { filename: 'name-rules.svelte.js', generate: 'client', dev: false });
  const directory = new URL('../.svelte-kit/', import.meta.url);
  await mkdir(directory, { recursive: true });
  const file = new URL(`name-rules-test-${randomUUID()}.mjs`, directory);
  await writeFile(file, compiled.js.code);
  let saved = [{ language: 'сербский', rules: 'Сохранённое правило' }];
  let finishSave;
  let sent;
  let deleted;
  let failSave = false;
  globalThis.__nameRulesInvoke = async (command, args) => {
    if (command === 'load_name_rules') return structuredClone(saved);
    assert.equal(command, 'save_name_rules');
    if (failSave) throw new Error('write failed');
    sent = args.rules;
    deleted = args.removed;
    await new Promise((resolve) => { finishSave = resolve; });
    saved = saved.filter((rule) => !args.removed.includes(rule.language));
    for (const rule of args.rules) {
      saved = saved.filter((own) => own.language !== rule.language);
      saved.push({ ...rule });
    }
  };
  try {
    const { nameRules, loadNameRules, setNameRule, saveNameRules, removeNameRule, restoreNameRule, renameNameRule, dirtyNameRuleLanguages, missingNameRuleLanguages } = await import(file.href);
    await loadNameRules();
    assert.deepEqual(dirtyNameRuleLanguages(), []);
    setNameRule(' Сербский ', 'Общий черновик из редактора');
    setNameRule('немецкий', 'Новый язык');
    assert.deepEqual(missingNameRuleLanguages([
      { languages: [' Сербский ', 'итальянский', 'описание', 'неизвестно', ''] },
      { languages: ['итальянский', 'немецкий'] }
    ]), ['итальянский']);
    await loadNameRules();
    assert.equal(nameRules.items.find((r) => r.language === 'сербский').rules, 'Общий черновик из редактора');
    assert.deepEqual(dirtyNameRuleLanguages(), ['сербский', 'немецкий']);

    const saving = saveNameRules();
    assert.equal(nameRules.saving, true);
    assert.equal(sent[0].rules, 'Общий черновик из редактора');
    setNameRule('сербский', 'Новая правка во время сохранения');
    finishSave();
    await saving;
    assert.deepEqual(dirtyNameRuleLanguages(), ['сербский']);
    assert.equal(nameRules.saving, false);

    failSave = true;
    await assert.rejects(saveNameRules(), /write failed/);
    assert.deepEqual(dirtyNameRuleLanguages(), ['сербский']);
    assert.equal(nameRules.saving, false);
    setNameRule('сербский', nameRules.saved['сербский']);
    assert.deepEqual(dirtyNameRuleLanguages(), []);

    renameNameRule('сербский', ' Сербохорватский ');
    assert.equal(nameRules.items.find((r) => r.language === 'сербохорватский').rules, 'Общий черновик из редактора');
    assert.throws(() => renameNameRule('сербохорватский', 'немецкий'), /уже существуют/);
    assert.throws(() => renameNameRule('сербохорватский', '  '), /Укажите язык/);
    removeNameRule('немецкий');
    assert.ok(dirtyNameRuleLanguages().includes('немецкий'));
    restoreNameRule('немецкий');
    assert.equal(nameRules.items.find((r) => r.language === 'немецкий').rules, 'Новый язык');
    removeNameRule('немецкий');
    await loadNameRules();
    assert.ok(!nameRules.items.some((r) => ['сербский', 'немецкий'].includes(r.language)));
    await assert.rejects(saveNameRules(), /write failed/);
    assert.deepEqual(nameRules.removed, ['сербский', 'немецкий']);

    failSave = false;
    const renamed = saveNameRules();
    assert.deepEqual(deleted, ['сербский', 'немецкий']);
    assert.deepEqual(sent, [{ language: 'сербохорватский', rules: 'Общий черновик из редактора' }]);
    finishSave();
    await renamed;
    assert.deepEqual(dirtyNameRuleLanguages(), []);
    await loadNameRules();
    assert.deepEqual(nameRules.items.map((r) => r.language), ['сербохорватский']);
    setNameRule('черновик', 'Не сохранён');
    removeNameRule('черновик');
    assert.deepEqual(dirtyNameRuleLanguages(), []);

    removeNameRule('сербохорватский');
    const removedAll = saveNameRules();
    assert.deepEqual(sent, []);
    finishSave();
    await removedAll;
    await loadNameRules();
    assert.deepEqual(nameRules.items, []);
    assert.deepEqual(dirtyNameRuleLanguages(), []);

  } finally {
    delete globalThis.__nameRulesInvoke;
    await unlink(file);
  }
});
