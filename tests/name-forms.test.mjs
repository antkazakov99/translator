// @ts-nocheck -- Node runtime tests use native TypeScript loading, outside the browser type environment.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { baseNames, canonicalForms, characterForms, characterTitle, defaultCanonicalName, followsCanonical, formNames, formOwners, formTranslation, fullName, hasScriptCode, knownNames, linkOptions, mainTranslation, nameTargetKey, newCharacterAlias, NO_CANONICAL_NAME, ownerOptions } from '../src/lib/name-forms.ts';

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
  const char = (alias, original_name, forms, last_name = '') => ({ ...NO_CANONICAL_NAME, alias, original_name, last_name, forms: forms.map(([source, translated = '']) => ({ source, translated })) });
  const found = characterForms(
    [
      char('em', 'Emma', [['Karanovic', 'Каранович'], ['Emma Karanovic'], [' ']]),
      char('ve', 'Vera', [[' karanovic ', 'Каранович'], ['Sister'], ['Boss', 'Шеф']], 'Karanovic'),
      char('k', 'Kate', [['Boss', 'Босс']])
    ],
    new Map([['karanovic', 30], ['sister', 2]])
  );
  assert.deepEqual(found.map((form) => [form.source, form.count]), [['Karanovic', 30], ['Sister', 2], ['Boss', 0], ['Emma Karanovic', 0]]);
  assert.deepEqual(found[0].owners, [{ alias: 'em', name: 'Emma' }, { alias: 've', name: 'Karanovic' }]);
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

test('a card shows the first and last name, then the name from the game, then the code', () => {
  assert.equal(characterTitle({ alias: 'lis', original_name: 'Liz', first_name: ' Lisa ', last_name: ' Smith ' }), 'Lisa Smith');
  assert.equal(characterTitle({ alias: 'lis', original_name: 'Liz', first_name: '', last_name: 'Smith' }), 'Smith');
  assert.equal(characterTitle({ alias: 'lis', original_name: 'Lisa', first_name: '  ', last_name: '' }), 'Lisa');
  assert.equal(characterTitle({ alias: 'lis', original_name: '', first_name: '', last_name: '' }), 'lis');
  assert.equal(fullName({ first_name: 'Lisa', last_name: ' ' }), 'Lisa');
});

const person = (alias, original_name, details = {}) => ({
  ...NO_CANONICAL_NAME, alias, original_name, translated_name: '', gender: 'female', forms: [], ...details
});

test('the canonical name becomes forms and leaves out empty parts', () => {
  assert.deepEqual(canonicalForms(person('l', 'Lisa', { first_name: ' Lisa ', translated_first_name: ' Лиза ' })), [{ source: 'Lisa', translated: 'Лиза' }]);
  assert.deepEqual(canonicalForms(person('l', 'Lisa')), []);
});

test('the canonical name shows up among forms unless it is the name from the game', () => {
  const lisa = person('l', 'Lisa', { first_name: 'Lisa', last_name: 'Smith', translated_last_name: 'Смит', forms: [{ source: 'smith', translated: '' }] });
  const found = characterForms([lisa], new Map());
  assert.deepEqual(found.map((form) => [form.source, form.owners.length, form.translations]), [['Smith', 1, ['Смит']]]);
});

test('the name from the game follows the canonical name it matches', () => {
  const details = { first_name: 'Lisa', last_name: 'Smith', translated_first_name: ' Лиза ', translated_last_name: 'Смит' };
  assert.equal(mainTranslation(person('l', 'lisa', details)), 'Лиза');
  assert.equal(mainTranslation(person('l', 'Smith', details)), 'Смит');
  assert.equal(mainTranslation(person('l', 'Lisa Smith', details)), 'Лиза Смит');
  assert.equal(mainTranslation(person('l', 'Lisa Smith', { ...details, translated_last_name: '' })), 'Лиза');
  assert.equal(followsCanonical(person('l', 'LISA SMITH', details)), true);
  assert.equal(followsCanonical(person('l', 'Mrs. S', details)), false);
  assert.equal(followsCanonical(person('l', '')), false);
  assert.equal(followsCanonical(person('l', '', details)), false);
});

test('the first name defaults to the name from the game with its translation', () => {
  assert.deepEqual(defaultCanonicalName(' Lisa ', ' Лиза '), { ...NO_CANONICAL_NAME, first_name: 'Lisa', translated_first_name: 'Лиза' });
});

test('the first step spells first and last names, and the name from the game only without them', () => {
  const batch = baseNames([
    person('a', 'Anna', { first_name: 'Anna', last_name: 'Kowalski', translated_first_name: 'Анна' }),
    person('p', 'Piotr', { first_name: 'Piotr', last_name: 'Kowalski', gender: 'male' }),
    person('z', 'Zofia', { last_name: 'kowalski' }),
    person('a2', 'ANNA', { translated_name: 'Аня' }),
    person('b', '', {})
  ]);
  assert.deepEqual(batch.requests.map(({ name, kind, gender }) => [name, kind, gender]), [
    ['Anna', 'name', 'female'],
    ['Kowalski', 'surname', 'female'],
    ['Piotr', 'name', 'male'],
    ['Kowalski', 'surname', 'male'],
    ['b', 'name', 'female']
  ]);
  const owners = batch.targets.get(batch.requests[1].id).map((target) => target.alias);
  assert.deepEqual(owners, ['a', 'z']);
  assert.deepEqual(batch.targets.get(batch.requests[0].id).map(({ alias, field, current }) => [alias, field, current]), [['a', 'first', 'Анна'], ['a2', 'main', 'Аня']]);
});

test('the second step builds forms on a filled name, then on the answer of the first step', () => {
  const anna = person('a', 'Mrs. K', {
    first_name: 'Anna',
    last_name: 'Karanovic',
    translated_first_name: 'Анна',
    forms: [{ source: 'Annie', translated: 'Анечка' }, { source: 'karanovic', translated: '' }, { source: ' ', translated: '' }, { source: 'annie', translated: '' }]
  });
  const kate = person('k', 'Kate', { translated_name: 'Кейт', forms: [{ source: 'Katie', translated: '' }] });
  const lisa = person('l', 'Lisa', { forms: [{ source: 'Liz', translated: '' }] });
  const spelled = new Map([
    [nameTargetKey({ alias: 'a', field: 'first', source: 'Anna' }), 'Анька'],
    [nameTargetKey({ alias: 'a', field: 'last', source: 'Karanovic' }), 'Каранович'],
    [nameTargetKey({ alias: 'l', field: 'main', source: 'Lisa' }), 'Лиза']
  ]);

  const batch = formNames([anna, kate, lisa], spelled);
  assert.deepEqual(batch.requests.map(({ name, kind, base }) => [name, kind, base]), [
    ['Mrs. K', 'form', 'Анна Каранович'],
    ['Annie', 'form', 'Анна Каранович'],
    ['Katie', 'form', 'Кейт'],
    ['Liz', 'form', 'Лиза']
  ]);
  const fields = [...batch.targets.values()].flat().map(({ field, current }) => [field, current]);
  assert.deepEqual(fields, [['main', ''], ['form', 'Анечка'], ['form', ''], ['form', '']]);
});

test('a name from the game equal to the full name is not a form', () => {
  const lisa = person('l', 'Lisa Smith', { first_name: 'Lisa', last_name: 'Smith' });
  assert.deepEqual(formNames([lisa], new Map()).requests, []);
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
