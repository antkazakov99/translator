// @ts-nocheck -- Node runtime tests use native TypeScript loading, outside the browser type environment.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { allNames, baseNames, canonicalForms, characterTitle, chatNames, cleanCharacterForms, cleanTermForms, defaultCanonicalName, formNames, formOwners, formTranslation, fullName, hasScriptCode, knownNames, nameTargetKey, newCharacterAlias, NO_CANONICAL_NAME, ownerOptions, translatedTitle, uniqueForms } from '../src/lib/name-forms.ts';

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

test('a card shows the first and last name, then the code', () => {
  assert.equal(characterTitle({ alias: 'lis', first_name: ' Lisa ', last_name: ' Smith ' }), 'Lisa Smith');
  assert.equal(characterTitle({ alias: 'lis', first_name: '', last_name: 'Smith' }), 'Smith');
  assert.equal(characterTitle({ alias: 'lis', first_name: '  ', last_name: '' }), 'lis');
  assert.equal(fullName({ first_name: 'Lisa', last_name: ' ' }), 'Lisa');
});

const person = (alias, first_name, details = {}) => ({
  ...NO_CANONICAL_NAME, alias, first_name, gender: 'female', merged: [], forms: [], ...details
});
const form = (source, translated = '') => ({ source, translated });

test('the translated title is the translated first and last name', () => {
  assert.equal(translatedTitle(person('k', 'Kira', { translated_first_name: 'Кира', translated_last_name: 'Ковач' })), 'Кира Ковач');
  assert.equal(translatedTitle(person('k', 'Kira')), '');
});

test('the canonical name becomes forms and leaves out empty parts', () => {
  assert.deepEqual(canonicalForms(person('l', ' Lisa ', { translated_first_name: ' Лиза ' })), [form('Lisa', 'Лиза')]);
  assert.deepEqual(canonicalForms(person('l', ' ')), []);
});

test('every spelling of a character is known, without blanks', () => {
  const lisa = person('l', 'Lisa', { last_name: 'Smith', merged: [{ alias: 'ls', original_name: 'Liz' }], forms: [form('Miss Smith'), form(' ')] });
  assert.deepEqual(allNames(lisa), ['Lisa Smith', 'Lisa', 'Smith', 'Liz', 'Miss Smith']);
});

test('a new character gets the first name with its translation', () => {
  assert.deepEqual(defaultCanonicalName(' Lisa ', ' Лиза '), { ...NO_CANONICAL_NAME, first_name: 'Lisa', translated_first_name: 'Лиза' });
});

test('forms lose blanks and repeats, and a repeat hands over its translation', () => {
  const { forms, names } = uniqueForms(
    [form(' '), form('PPF'), form('ppf ', 'ППФ'), form('Academy', 'Академия'), form('ACADEMY', 'Другое'), form('Foundation', 'Фонд')],
    [form('Foundation'), form('', '')]
  );
  assert.deepEqual(forms, [form('PPF', 'ППФ'), form('Academy', 'Академия')]);
  assert.deepEqual(names, [form('Foundation', 'Фонд'), form('', '')]);
});

test('a character form copying the first, last or full name is removed', () => {
  const anna = person('a', 'Anna', {
    last_name: 'Karanovic',
    translated_first_name: 'Анна',
    merged: [{ alias: 'a2', original_name: 'Ann' }],
    forms: [form('anna', 'Другое'), form('Karanovic', 'Каранович'), form(' ANNA KARANOVIC '), form('ann'), form('Annie', 'Энни')]
  });
  cleanCharacterForms(anna);
  assert.deepEqual(anna.forms, [form('Annie', 'Энни')]);
  assert.equal(anna.translated_first_name, 'Анна');
  assert.equal(anna.translated_last_name, 'Каранович');
});

test('a merged record without a script code becomes a form, a real code stays merged', () => {
  const stu = person('~stu', 'Stu', {
    merged: [{ alias: '~stuart', original_name: 'Stuart' }, { alias: 'st', original_name: 'Stu Jr' }, { alias: '~stu_2', original_name: 'stu' }],
    forms: [form('Stuart', 'Стюарт')]
  });
  cleanCharacterForms(stu);
  assert.deepEqual(stu.merged, [{ alias: 'st', original_name: 'Stu Jr' }]);
  assert.deepEqual(stu.forms, [form('Stuart', 'Стюарт')]);

  const bare = person('b', 'Bob', { merged: [{ alias: '~bobby', original_name: 'Bobby' }] });
  cleanCharacterForms(bare);
  assert.deepEqual([bare.merged, bare.forms], [[], [form('Bobby')]]);
});

test('a removed full name fills the empty fields when it can be split', () => {
  const full = (translated, details = {}) => person('a', 'Anna', { last_name: 'Karanovic', forms: [form('Anna Karanovic', translated)], ...details });
  const anna = full('Анна Каранович', { translated_first_name: 'Аня' });
  cleanCharacterForms(anna);
  assert.deepEqual([anna.forms, anna.translated_first_name, anna.translated_last_name], [[], 'Аня', 'Каранович']);

  const unclear = full('Анна фон Каранович');
  cleanCharacterForms(unclear);
  assert.deepEqual([unclear.translated_first_name, unclear.translated_last_name], ['', '']);
});

test('a form copying its term is removed and keeps the translation', () => {
  const term = { source_term: 'PPF', translated_term: '', forms: [form('ppf', 'ППФ'), form('Foundation')] };
  cleanTermForms(term);
  assert.equal(term.translated_term, 'ППФ');
  assert.deepEqual(term.forms, [form('Foundation')]);
});

test('the first step spells first and last names, a surname for each gender', () => {
  const batch = baseNames([
    person('a', 'Anna', { last_name: 'Kowalski', translated_first_name: 'Анна' }),
    person('p', 'Piotr', { last_name: 'Kowalski', gender: 'male' }),
    person('z', 'Zofia', { last_name: 'kowalski' }),
    person('a2', 'ANNA', { translated_first_name: 'Аня' }),
    person('b', ' ')
  ]);
  assert.deepEqual(batch.requests.map(({ name, kind, gender }) => [name, kind, gender]), [
    ['Anna', 'name', 'female'],
    ['Kowalski', 'surname', 'female'],
    ['Piotr', 'name', 'male'],
    ['Kowalski', 'surname', 'male'],
    ['Zofia', 'name', 'female']
  ]);
  const owners = batch.targets.get(batch.requests[1].id).map((target) => target.alias);
  assert.deepEqual(owners, ['a', 'z']);
  assert.deepEqual(batch.targets.get(batch.requests[0].id).map(({ alias, field, current }) => [alias, field, current]), [['a', 'first', 'Анна'], ['a2', 'first', 'Аня']]);
});

test('the second step builds forms on a filled name, then on the answer of the first step', () => {
  const anna = person('a', 'Anna', {
    last_name: 'Karanovic',
    translated_first_name: 'Анна',
    forms: [form('Mrs. K'), form('Annie', 'Анечка'), form('karanovic'), form(' '), form('annie')]
  });
  const lisa = person('l', 'Lisa', { forms: [form('Liz')] });
  const spelled = new Map([
    [nameTargetKey({ alias: 'a', field: 'first', source: 'Anna' }), 'Анька'],
    [nameTargetKey({ alias: 'a', field: 'last', source: 'Karanovic' }), 'Каранович'],
    [nameTargetKey({ alias: 'l', field: 'first', source: 'Lisa' }), 'Лиза']
  ]);

  const batch = formNames([anna, lisa], spelled);
  assert.deepEqual(batch.requests.map(({ name, kind, base }) => [name, kind, base]), [
    ['Mrs. K', 'form', 'Анна Каранович'],
    ['Annie', 'form', 'Анна Каранович'],
    ['Liz', 'form', 'Лиза']
  ]);
  const fields = [...batch.targets.values()].flat().map(({ field, current }) => [field, current]);
  assert.deepEqual(fields, [['form', ''], ['form', 'Анечка'], ['form', '']]);
});

test('a chat bot gets both steps at once, forms built on the names it spells', () => {
  const anna = person('a', 'Anna', { last_name: 'Karanovic', translated_first_name: 'Анна', forms: [form('Mrs. K')] });
  const lisa = person('l', 'Lisa', { forms: [form('Liz')] });

  const batch = chatNames([anna, lisa]);
  assert.deepEqual(batch.requests.map(({ id, name, kind, base }) => [id, name, kind, base]), [
    ['n0', 'Anna', 'name', ''],
    ['n1', 'Karanovic', 'surname', ''],
    ['n2', 'Lisa', 'name', ''],
    ['f0', 'Mrs. K', 'form', 'Анна Karanovic'],
    ['f1', 'Liz', 'form', 'Lisa']
  ]);
  assert.deepEqual(batch.targets.get('f1').map(({ alias, field, source }) => [alias, field, source]), [['l', 'form', 'Liz']]);
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
