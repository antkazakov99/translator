import assert from "node:assert/strict";
import { test } from "node:test";
import { hasWords } from "../src/lib/line-text.ts";

const tokens = [
  { name: "mc", value: "" },
  { name: "roommate", value: "roommate" }
];

test("реплика из токенов без значения и тегов не содержит слов", () => {
  for (const text of ["...", "[mc]!", "[mc!c]... [unknown]?", "{i}[mc]{/i}!", "{w=0.5}..."]) {
    assert.equal(hasWords(text, tokens), false, text);
  }
});

test("токен со значением считается словом", () => {
  assert.equal(hasWords("[roommate]!", tokens), true);
  assert.equal(hasWords("{i}[roommate!c]{/i}", tokens), true);
});

test("обычный текст рядом с токеном остаётся словами", () => {
  assert.equal(hasWords("Hi, [mc]!", tokens), true);
  assert.equal(hasWords("[mc]'s", tokens), true);
});
