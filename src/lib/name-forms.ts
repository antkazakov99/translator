/** Другая форма записи глоссария: сокращение термина или обращение к персонажу. */
export interface NameForm {
  source: string;
  translated: string;
}

/** Запись глоссария, которой может принадлежать форма. */
export interface FormOwner {
  kind: "term" | "character";
  id: string;
  /** Имена, с которыми сравнивается форма. Первое показывается пользователю. */
  names: string[];
  /** Переводы и другие формы. Их находят поиск и подсказка, но имена записи показываются только из `names`. */
  linked: string[];
}

export const ownerKey = (owner: Pick<FormOwner, "kind" | "id">) => `${owner.kind}:${owner.id}`;

// Такого знака не бывает в коде Ren'Py, поэтому внутренний номер не совпадёт с настоящим кодом
const NO_CODE = "~";

/** Подпись карточки персонажа: своё имя карточки, иначе имя из игры. */
export const characterTitle = (char: { alias: string; original_name: string; card_name: string }) =>
  char.card_name.trim() || char.original_name.trim() || char.alias;

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
 * ещё не значат одно и то же, поэтому связывает записи пользователь.
 */
export function formOwners(form: string, owners: FormOwner[]) {
  const source = form.trim();
  return owners.filter(({ names, linked }) => {
    const own = [...names, ...linked].map((name) => name.trim()).filter(Boolean);
    if (own.some((name) => name.toLowerCase() === source.toLowerCase())) return false;
    return own.some((name) => matchesName(source, name));
  });
}

/**
 * Имена для подсказок модели. Одно написание может быть у нескольких записей, например фамилия:
 * пол тогда неизвестен, а если расходится и перевод, подсказка только запутала бы модель.
 */
export function knownNames<T extends { original: string; translated: string; gender: string }>(names: T[]) {
  const same = new Map<string, T[]>();
  for (const name of names) {
    const key = name.original.trim().toLowerCase();
    same.set(key, [...(same.get(key) ?? []), name]);
  }
  return [...same.values()].flatMap((group) => {
    const [first] = group;
    if (group.some((name) => name.translated !== first.translated)) return [];
    return [group.every((name) => name.gender === first.gender) ? first : { ...first, gender: "unknown" }];
  });
}

/** Пункты поиска записи: сначала похожие, затем персонажи и термины. Запись показывается один раз. */
export function ownerOptions(query: string, suggested: FormOwner[], characters: FormOwner[], terms: FormOwner[]) {
  const words = query.trim().toLowerCase();
  const found = (owners: FormOwner[]) =>
    owners.filter(({ names, linked }) => [...names, ...linked].some((name) => name.toLowerCase().includes(words)));
  const rest = (owners: FormOwner[]) => found(owners).filter((owner) => !suggested.some((own) => ownerKey(own) === ownerKey(owner)));
  const suggestedName = (owner: FormOwner) => (owner.kind === "character" ? `Персонаж ${owner.names[0]}` : owner.names[0]);
  return [
    ...found(suggested).map((owner) => ({ id: ownerKey(owner), name: owner.names[0], label: suggestedName(owner), group: "Похоже" })),
    ...rest(characters).map((owner) => ({ id: ownerKey(owner), name: owner.names[0], label: owner.names[0], group: "Персонажи" })),
    ...rest(terms).map((owner) => ({ id: ownerKey(owner), name: owner.names[0], label: owner.names[0], group: "Термины" }))
  ];
}

/** Пункты выбора нескольких персонажей: связанные всегда сверху, чтобы было видно, с кем связана форма. */
export function linkOptions(query: string, suggested: FormOwner[], characters: FormOwner[], linked: string[]) {
  const options = ownerOptions(query, suggested, characters, []);
  return [
    ...options.filter((option) => linked.includes(option.id)).map((option) => ({ ...option, group: "Связано" })),
    ...options.filter((option) => !linked.includes(option.id))
  ];
}

interface FormCharacter {
  alias: string;
  original_name: string;
  card_name: string;
  forms: NameForm[];
}

/**
 * Формы персонажей, по одной на написание: фамилия бывает общей. Отдельно они нигде не хранятся.
 * Сначала идут самые частые, а `counts` даёт число строк игры с формой в нижнем регистре.
 */
export function characterForms(characters: FormCharacter[], counts: Map<string, number>) {
  const found = new Map<string, { key: string; source: string; count: number; owners: { alias: string; name: string }[]; translations: string[] }>();
  for (const char of characters) {
    for (const form of char.forms) {
      const source = form.source.trim();
      if (!source) continue;
      const key = source.toLowerCase();
      const entry = found.get(key) ?? { key, source, count: counts.get(key) ?? 0, owners: [], translations: [] };
      entry.owners.push({ alias: char.alias, name: characterTitle(char) });
      if (form.translated.trim() && !entry.translations.includes(form.translated)) entry.translations.push(form.translated);
      found.set(key, entry);
    }
  }
  return [...found.values()].sort((a, b) => b.count - a.count || a.source.localeCompare(b.source));
}

/** Форма записи и перевод, который она получит: если своего нет, берётся перевод записи. */
export const formTranslation = (form: NameForm, main: string) => form.translated.trim() || main.trim();
