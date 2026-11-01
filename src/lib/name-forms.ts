/** Другая форма записи глоссария: сокращение термина или обращение к персонажу. */
export interface NameForm {
  source: string;
  translated: string;
}

/** Запись глоссария, которой может принадлежать форма. */
export interface FormOwner {
  kind: "term" | "character" | "found";
  id: string;
  /** Имена, с которыми сравнивается форма. Первое показывается пользователю. */
  names: string[];
}

/** Значение в выборе записи: вместо привязки создать персонажа. */
export const NEW_CHARACTER = "new-character";

export const ownerKey = (owner: Pick<FormOwner, "kind" | "id">) => `${owner.kind}:${owner.id}`;

// Такого знака не бывает в коде Ren'Py, поэтому внутренний номер не совпадёт с настоящим кодом
const NO_CODE = "~";

/** Есть ли у персонажа настоящий код из скриптов. Показывать можно только его. */
export const hasScriptCode = (alias: string) => !alias.startsWith(NO_CODE);

/**
 * Внутренний номер персонажа без кода: его только упоминают, и в скриптах кода нет.
 * Записи глоссария различаются по `alias`, поэтому пустым он быть не может.
 */
export function newCharacterAlias(name: string, taken: string[]) {
  const words = name.trim().toLowerCase().replace(/[^\p{L}\p{N}]+/gu, "_").replace(/^_+|_+$/g, "") || "character";
  const base = `${NO_CODE}${words}`;
  const used = new Set(taken.map((alias) => alias.toLowerCase()));
  let alias = base;
  for (let number = 2; used.has(alias); number += 1) alias = `${base}_${number}`;
  return alias;
}

const LINK_WORDS = new Set(["of", "the", "de", "la", "del", "von", "der", "and"]);
const ABBREVIATION = /^\p{Lu}{2,6}$/u;

const wordsOf = (name: string) => name.trim().split(/\s+/).filter(Boolean);
const initials = (words: string[]) => words.map((word) => word[0]).join("").toUpperCase();

function matchesName(form: string, name: string) {
  const words = wordsOf(name);
  if (ABBREVIATION.test(form)) {
    const meaningful = words.filter((word) => !LINK_WORDS.has(word.toLowerCase()));
    return words.length > 1 && (initials(meaningful) === form || initials(words) === form);
  }
  const formWords = wordsOf(form).map((word) => word.toLowerCase());
  const nameWords = words.map((word) => word.toLowerCase());
  if (formWords.length === 1) return formWords[0].length >= 3 && nameWords.includes(formWords[0]);
  // У «Miss Granger» и «Hermione Granger» общая фамилия, а у «Silver Key» и «Silver Sword» только признак
  return formWords.at(-1) === nameWords.at(-1);
}

/**
 * Записи, формой которых похоже выступает `form`. Это только подсказка: одинаковые буквы и слова
 * ещё не значат одно и то же, поэтому связывает записи пользователь. `shortFor` это мнение модели.
 */
export function formOwners(form: string, owners: FormOwner[], shortFor = "") {
  const source = form.trim();
  const named = shortFor.trim().toLowerCase();
  return owners.filter(({ names }) => {
    const own = names.map((name) => name.trim()).filter(Boolean);
    if (own.some((name) => name.toLowerCase() === source.toLowerCase())) return false;
    return own.some((name) => name.toLowerCase() === named || matchesName(source, name));
  });
}

/** Форма записи и перевод, который она получит: если своего нет, берётся перевод записи. */
export const formTranslation = (form: NameForm, main: string) => form.translated.trim() || main.trim();
