import assert from "node:assert/strict";
import test from "node:test";
import { languageList, untranslatedNames, withSpelledNames } from "../src/lib/glossary.ts";

test("language codes read as Russian language names", () => {
  assert.equal(languageList(["sr", "de", "zxx", "und"]), "сербский, немецкий, описание, неизвестно");
  assert.equal(languageList(["qqq"]), "qqq", "неизвестный код остаётся кодом");
});

const anna = {
  id: 1,
  firstName: "Anna",
  lastName: "Karanovic",
  translatedFirstName: null,
  translatedLastName: "Каранович",
  gender: "female",
  speech: null,
  description: null,
  forms: [
    { text: "Mrs. Karanovic", translation: null },
    { text: "Annie", translation: "Энни" },
  ],
};

test("untranslated names count first and last names and forms without translation", () => {
  assert.equal(untranslatedNames(anna), 2);
  assert.equal(untranslatedNames({ ...anna, firstName: null, forms: [] }), 0);
});

test("spelled names fill only empty fields and find the form by its text", () => {
  const next = withSpelledNames(anna, [
    { field: "first", source: "Anna", translation: " Анна " },
    { field: "last", source: "Karanovic", translation: "Карановиц" },
    { field: "form", source: "mrs. karanovic", translation: "Миссис Каранович" },
    { field: "form", source: "Annie", translation: "Анни" },
  ]);
  assert.equal(next.translatedFirstName, "Анна");
  assert.equal(next.translatedLastName, "Каранович", "перевод из карточки не заменяется");
  assert.deepEqual(next.forms, [
    { text: "Mrs. Karanovic", translation: "Миссис Каранович" },
    { text: "Annie", translation: "Энни" },
  ]);
  assert.equal(anna.forms[0].translation, null, "исходный персонаж не меняется");
  assert.equal(withSpelledNames(anna, [{ field: "last", source: "Karanovic", translation: "Карановиц" }]), null, "писать нечего");
});
