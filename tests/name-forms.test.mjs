// @ts-nocheck -- Node runtime tests use native TypeScript loading, outside the browser type environment.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { formOwners, formTranslation, hasScriptCode, newCharacterAlias } from '../src/lib/name-forms.ts';

const term = (name) => ({ kind: 'term', id: name, names: [name] });
const owners = (found) => found.map((owner) => owner.id);

test('an abbreviation matches the initials of a longer name', () => {
  const list = [term('Pleasure Palace Foundation'), term('Order of the Silver Dawn'), term('Pizza')];
  assert.deepEqual(owners(formOwners('PPF', list)), ['Pleasure Palace Foundation']);
  assert.deepEqual(owners(formOwners('OSD', list)), ['Order of the Silver Dawn']);
  assert.deepEqual(owners(formOwners('OOTSD', list)), ['Order of the Silver Dawn']);
  assert.deepEqual(owners(formOwners('XY', list)), []);
});

test('an address to a character finds the character by surname or first name', () => {
  const hermione = { kind: 'character', id: 'her', names: ['Hermione Granger'] };
  const harry = { kind: 'character', id: 'har', names: ['Harry Potter'] };
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
