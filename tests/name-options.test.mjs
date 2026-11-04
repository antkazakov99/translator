import { test } from 'node:test';
import assert from 'node:assert/strict';
import { nameOptions, nameCheckNotice, modelNameChoice } from '../src/lib/name-options.ts';

test('choices preserve the glossary spelling, deduplicate variants and keep explanations short', () => {
  const options = nameOptions({ translated: 'Хорватова', variants: [
    { translated: 'Хорват', description: 'Совпадает с глоссарием' },
    { translated: 'Хорватова', description: '**Вариант**\n' + 'длинное пояснение '.repeat(100) },
    { translated: 'Хорватова', description: 'Повтор' }
  ] }, 'Хорват');
  assert.deepEqual(options.map((option) => option.translated), ['Хорват', 'Хорватова']);
  assert.equal(options[0].description, 'Совпадает с глоссарием');
  assert.ok(options[1].description.length <= 180);
  assert.ok(!options[1].description.includes('*'));
});

test('a saved spelling keeps its own explanation even when the model recommends another option', () => {
  const options = nameOptions({ translated: 'Марлен', variants: [{ translated: 'Марлен', description: 'По французскому произношению.' }], glossary_variants: [{ translated: 'Марлена', description: 'Чтение с произносимой конечной гласной.' }] }, 'Марлена');
  assert.equal(options[0].translated, 'Марлена');
  assert.equal(options[0].description, 'Чтение с произносимой конечной гласной.');
});

test('verified alternatives replace an erroneous initial suggestion and warnings become one short status', () => {
  assert.deepEqual(nameOptions({ translated: 'Хорватова', variants: [{ translated: 'Хорват', description: 'Сохранена форма фамилии.' }] }, '').map((option) => option.translated), ['Хорват']);
  assert.equal(nameCheckNotice(['Происхождение неоднозначно; проверьте выбор написания.']), '');
  assert.equal(nameCheckNotice(['Запрос проверки не удался; написание не проверено.']), 'Проверка не завершена.');
});

test('choosing the model recommendation always selects an available radio option', () => {
  const suggestion = { translated: 'Хорватова', variants: [{ translated: ' Хорват ', description: 'Исправлено проверкой' }] };
  const options = nameOptions(suggestion, 'Хорват');
  assert.equal(options.length, 1);
  assert.equal(modelNameChoice(suggestion, options), 'Хорват');
  const single = nameOptions({ translated: ' Аленка ' }, '');
  assert.equal(modelNameChoice({ translated: ' Аленка ' }, single), 'Аленка');
  const unavailable = modelNameChoice({ translated: 'Невидимый вариант' }, options);
  assert.ok(options.some((option) => option.translated === unavailable));
});
