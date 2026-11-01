// @ts-nocheck -- Node runtime tests use native TypeScript loading, outside the browser type environment.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { characterForms, characterTitle, formOwners, formTranslation, hasScriptCode, knownNames, linkOptions, newCharacterAlias, ownerOptions } from '../src/lib/name-forms.ts';

const term = (name, linked = []) => ({ kind: 'term', id: name, names: [name], linked });
const owners = (found) => found.map((owner) => owner.id);

test('an abbreviation matches the initials of a longer name', () => {
  const list = [term('Pleasure Palace Foundation'), term('Order of the Silver Dawn'), term('Pizza')];
  assert.deepEqual(owners(formOwners('PPF', list)), ['Pleasure Palace Foundation']);
  assert.deepEqual(owners(formOwners('OSD', list)), ['Order of the Silver Dawn']);
  assert.deepEqual(owners(formOwners('OOTSD', list)), ['Order of the Silver Dawn']);
  assert.deepEqual(owners(formOwners('XY', list)), []);
});

test('an address to a character finds the character by surname or first name', () => {
  const hermione = { kind: 'character', id: 'her', names: ['Hermione Granger'], linked: [] };
  const harry = { kind: 'character', id: 'har', names: ['Harry Potter'], linked: [] };
  assert.deepEqual(owners(formOwners('Miss Granger', [hermione, harry])), ['her']);
  assert.deepEqual(owners(formOwners('Hermione', [hermione, harry])), ['her']);
  assert.deepEqual(owners(formOwners('Miss Weasley', [hermione, harry])), []);
});

test('a short form finds its full name, but a shared adjective does not', () => {
  assert.deepEqual(owners(formOwners('Academy', [term('Ninja Academy')])), ['Ninja Academy']);
  assert.deepEqual(owners(formOwners('Silver Key', [term('Silver Sword')])), []);
});

test('a record is never a form of itself', () => {
  assert.deepEqual(formOwners('Ninja Academy', [term('Ninja Academy')]), []);
  assert.deepEqual(formOwners('ppf', [term('PPF')]), []);
});

test('a form shared by characters is one entry, the most frequent go first', () => {
  const char = (alias, original_name, forms, card_name = '') => ({ alias, original_name, card_name, forms: forms.map(([source, translated = '']) => ({ source, translated })) });
  const found = characterForms(
    [
      char('em', 'Emma', [['Karanovic', 'Каранович'], ['Emma Karanovic'], [' ']]),
      char('ve', 'Vera', [[' karanovic ', 'Каранович'], ['Sister'], ['Boss', 'Шеф']], 'Vera K'),
      char('k', 'Kate', [['Boss', 'Босс']])
    ],
    new Map([['karanovic', 30], ['sister', 2]])
  );
  assert.deepEqual(found.map((form) => [form.source, form.count]), [['Karanovic', 30], ['Sister', 2], ['Boss', 0], ['Emma Karanovic', 0]]);
  assert.deepEqual(found[0].owners, [{ alias: 'em', name: 'Emma' }, { alias: 've', name: 'Vera K' }]);
  assert.deepEqual(found[0].translations, ['Каранович']);
  assert.deepEqual(found[2].translations, ['Шеф', 'Босс']);
});

test('linked characters stay on top of the picker while it searches', () => {
  const char = (id, name) => ({ kind: 'character', id, names: [name], linked: [] });
  const [emma, vera, kate] = [char('em', 'Emma'), char('ve', 'Vera'), char('k', 'Kate')];
  const list = linkOptions('', [vera], [emma, vera, kate], ['character:k']);
  assert.deepEqual(list.map((option) => [option.group, option.name]), [['Связано', 'Kate'], ['Похоже', 'Vera'], ['Персонажи', 'Emma']]);
  assert.deepEqual(linkOptions('em', [vera], [emma, vera, kate], ['character:k']).map((option) => option.name), ['Emma']);
});

test('a shared surname suggests every record until it is linked to them', () => {
  const lisa = { kind: 'character', id: 'l', names: ['Lisa Smith'], linked: [] };
  const bob = { kind: 'character', id: 'b', names: ['Bob Smith'], linked: [] };
  assert.deepEqual(owners(formOwners('Smith', [lisa, bob])), ['l', 'b']);
  assert.deepEqual(owners(formOwners('Smith', [{ ...lisa, linked: ['смит', ' smith '] }, bob])), ['b']);
});

test('a name shared by records keeps its translation but not a gender, and a disputed one is dropped', () => {
  const name = (original, translated, gender) => ({ original, translated, gender });
  assert.deepEqual(knownNames([name('Kate', 'Кейт', 'female')]), [name('Kate', 'Кейт', 'female')]);
  assert.deepEqual(
    knownNames([name('Smith', 'Смит', 'female'), name('smith', 'Смит', 'male')]),
    [name('Smith', 'Смит', 'unknown')]
  );
  assert.deepEqual(knownNames([name('Smith', 'Смит', 'female'), name('Smith', 'Смит', 'female')]), [name('Smith', 'Смит', 'female')]);
  assert.deepEqual(knownNames([name('Smith', 'Лиза', 'female'), name('Smith', 'Боб', 'male')]), []);
});

test('a card shows its own name, then the name from the game, then the code', () => {
  assert.equal(characterTitle({ alias: 'lis', original_name: 'Lisa', card_name: ' Lisa Smith ' }), 'Lisa Smith');
  assert.equal(characterTitle({ alias: 'lis', original_name: 'Lisa', card_name: '  ' }), 'Lisa');
  assert.equal(characterTitle({ alias: 'lis', original_name: '', card_name: '' }), 'lis');
});

test('the owner search keeps groups in order and shows a suggested record once', () => {
  const hermione = { kind: 'character', id: 'her', names: ['Hermione Granger'], linked: [] };
  const harry = { kind: 'character', id: 'har', names: ['Harry Potter'], linked: [] };
  const academy = term('Ninja Academy');
  const list = ownerOptions('', [hermione], [hermione, harry], [academy]);
  assert.deepEqual(list.map((option) => [option.group, option.label]), [
    ['Похоже', 'Персонаж Hermione Granger'],
    ['Персонажи', 'Harry Potter'],
    ['Термины', 'Ninja Academy']
  ]);
  assert.equal(list[1].id, 'character:har');
});

test('the owner search finds a record by its translation and other forms', () => {
  const foundation = term('Pleasure Palace Foundation', ['Фонд', 'PPF', 'ФДН']);
  const hermione = { kind: 'character', id: 'her', names: ['Hermione Granger'], linked: ['Гермиона', 'Miss Granger', 'Мисс Грейнджер'] };
  const labels = (query) => ownerOptions(query, [], [hermione], [foundation]).map((option) => option.label);
  assert.deepEqual(labels('ppf'), ['Pleasure Palace Foundation']);
  assert.deepEqual(labels('мисс'), ['Hermione Granger']);
  assert.deepEqual(labels('гермиона'), ['Hermione Granger']);
});

test('a surname written only in other names still suggests every record', () => {
  const emma = { kind: 'character', id: 'em', names: ['Emma'], linked: ['Эмма', 'Emma Karanovic', ''] };
  const vera = { kind: 'character', id: 've', names: ['Vera'], linked: ['Вера', 'Vera Karanovic', 'Detective-Colonel Karanovic', ''] };
  assert.deepEqual(owners(formOwners('Karanovic', [emma, vera])), ['em', 've']);
  assert.deepEqual(owners(formOwners('Miss Karanovic', [emma, vera])), ['em', 've']);
  assert.deepEqual(owners(formOwners('Karanovic', [{ ...emma, linked: [...emma.linked, 'Karanovic'] }, vera])), ['ve']);
  assert.deepEqual(owners(formOwners('Petrovic', [emma, vera])), []);
});

test('the owner search looks through every name of a record', () => {
  const kate = { kind: 'character', id: 'k', names: ['Kate', 'Katherine Bell'], linked: [] };
  assert.deepEqual(ownerOptions(' BELL ', [], [kate], [term('Bell Tower')]).map((option) => option.label), ['Kate', 'Bell Tower']);
  assert.deepEqual(ownerOptions('nobody', [], [kate], []), []);
});

test('a character without a script code gets a free inner id that never looks like a code', () => {
  assert.equal(newCharacterAlias(' Miss Granger ', []), '~miss_granger');
  assert.equal(newCharacterAlias("Dr. O'Neil", []), '~dr_o_neil');
  assert.equal(newCharacterAlias('Гермиона', []), '~гермиона');
  assert.equal(newCharacterAlias('Kate', ['kate', '~KATE', '~kate_2']), '~kate_3');
  assert.equal(newCharacterAlias('!!!', []), '~character');

  assert.equal(hasScriptCode(newCharacterAlias('Kate', [])), false);
  assert.equal(hasScriptCode('kate'), true);
});

test('a form takes the translation of its record when it has none', () => {
  assert.equal(formTranslation({ source: 'PPF', translated: ' ППФ ' }, 'Фонд'), 'ППФ');
  assert.equal(formTranslation({ source: 'PPF', translated: ' ' }, ' Фонд '), 'Фонд');
});
