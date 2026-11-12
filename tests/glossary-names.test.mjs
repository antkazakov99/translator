import assert from "node:assert/strict";
import test from "node:test";
import { languageList, untranslatedNames } from "../src/lib/glossary.ts";

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
