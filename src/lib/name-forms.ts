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

/** Каноничные имя и фамилия персонажа. Они необязательны: у роли вроде Old Bartender их нет. */
export interface CanonicalName {
  first_name: string;
  last_name: string;
  translated_first_name: string;
  translated_last_name: string;
}

export const NO_CANONICAL_NAME: CanonicalName = {
  first_name: "",
  last_name: "",
  translated_first_name: "",
  translated_last_name: ""
};

const joinName = (...parts: string[]) => parts.map((part) => part.trim()).filter(Boolean).join(" ");
const sameName = (one: string, other: string) => one.trim().toLowerCase() === other.trim().toLowerCase();

/** Имя и фамилия одной строкой. */
export const fullName = (char: Pick<CanonicalName, "first_name" | "last_name">) => joinName(char.first_name, char.last_name);

/** Заполненные части каноничного имени в виде форм. */
export function canonicalForms(char: CanonicalName): NameForm[] {
  return [
    { source: char.first_name.trim(), translated: char.translated_first_name.trim() },
    { source: char.last_name.trim(), translated: char.translated_last_name.trim() }
  ].filter((form) => form.source);
}

/** Подпись карточки персонажа: имя с фамилией, иначе код. */
export const characterTitle = (char: { alias: string; first_name: string; last_name: string }) => fullName(char) || char.alias;

/** Перевод подписи карточки. */
export const translatedTitle = (char: CanonicalName) => joinName(char.translated_first_name, char.translated_last_name);

/** Имя нового персонажа. Фамилию вносит пользователь. */
export const defaultCanonicalName = (name: string, translated: string): CanonicalName => ({
  ...NO_CANONICAL_NAME,
  first_name: name.trim(),
  translated_first_name: translated.trim()
});

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

interface FormCharacter extends CanonicalName {
  alias: string;
  merged: { alias: string; original_name: string }[];
  forms: NameForm[];
}

/** Все написания персонажа: по ним он находится в поиске и среди говорящих без кода. */
export const allNames = (char: FormCharacter) =>
  [fullName(char), char.first_name, char.last_name, ...char.merged.map((part) => part.original_name), ...char.forms.map((form) => form.source)]
    .map((name) => name.trim())
    .filter(Boolean);

/** Форма записи и перевод, который она получит: если своего нет, берётся перевод записи. */
export const formTranslation = (form: NameForm, main: string) => form.translated.trim() || main.trim();

/**
 * Формы без пустых строк и повторов: написание сравнивается с именами записи `names` и с более ранними
 * формами. Перевод лишней формы не теряется, если у оставшейся записи его нет.
 */
export function uniqueForms(forms: NameForm[], names: NameForm[]) {
  const own = names.map((name) => ({ ...name }));
  const kept: NameForm[] = [];
  for (const form of forms) {
    if (!form.source.trim()) continue;
    const same = [...own, ...kept].filter((name) => sameName(name.source, form.source));
    if (same.length === 0) kept.push({ ...form });
    for (const name of same) {
      if (!name.translated.trim()) name.translated = form.translated;
    }
  }
  return { forms: kept, names: own };
}

/**
 * Убирает повторы форм персонажа, иначе одно написание получит два перевода и подсказка модели пропадёт.
 * Запись без кода в скриптах вошла в персонажа только именем, поэтому она становится формой.
 */
export function cleanCharacterForms(char: FormCharacter) {
  const noCode = char.merged.filter((part) => !hasScriptCode(part.alias));
  char.merged = char.merged.filter((part) => hasScriptCode(part.alias));
  const allForms = [...char.forms, ...noCode.map((part) => ({ source: part.original_name, translated: "" }))];
  const { forms, names } = uniqueForms(allForms, [
    { source: char.first_name, translated: char.translated_first_name },
    { source: char.last_name, translated: char.translated_last_name },
    { source: fullName(char), translated: "" },
    ...char.merged.map((part) => ({ source: part.original_name, translated: "" }))
  ]);
  char.forms = forms;
  const [first, last, full] = names.map((name) => name.translated);
  // Перевод полного имени нужен полям, пока те пусты, но разделить можно только два слова
  const words = wordsOf(full);
  const split = words.length === 2 && char.first_name.trim() && char.last_name.trim();
  char.translated_first_name = first.trim() || (split ? words[0] : first);
  char.translated_last_name = last.trim() || (split ? words[1] : last);
}

/** Убирает повторы форм термина. */
export function cleanTermForms(term: { source_term: string; translated_term: string; forms: NameForm[] }) {
  const { forms, names } = uniqueForms(term.forms, [{ source: term.source_term, translated: term.translated_term }]);
  term.forms = forms;
  term.translated_term = names[0].translated;
}

/** Поле персонажа, в которое попадёт написание от модели. */
export type NameField = "first" | "last" | "form";

/** Имя в запросе к модели. Одинаковые запросы объединены: написание нужно всем их `NameTarget`. */
export interface NameRequest {
  id: string;
  name: string;
  kind: "name" | "surname" | "form";
  gender: string;
  base: string;
}

/** Куда записать написание: у персонажа `alias` поле `field`, а у формы её `source`. */
export interface NameTarget {
  alias: string;
  field: NameField;
  source: string;
  current: string;
  base: string;
}

export interface NameBatch {
  requests: NameRequest[];
  targets: Map<string, NameTarget[]>;
}

interface NamedCharacter extends FormCharacter {
  gender: string;
}

export const nameTargetKey = (target: Pick<NameTarget, "alias" | "field" | "source">) =>
  `${target.alias}:${target.field}:${target.source.toLowerCase()}`;

type WantedName = NameTarget & Pick<NameRequest, "kind" | "gender">;

function nameBatch(wanted: WantedName[], prefix: string): NameBatch {
  const batch: NameBatch = { requests: [], targets: new Map() };
  const ids = new Map<string, string>();
  const seen = new Set<string>();
  for (const { kind, gender, ...target } of wanted) {
    if (seen.has(nameTargetKey(target))) continue;
    seen.add(nameTargetKey(target));
    // Пол важен только фамилии и форме: у остальных одно написание на всех
    const key = JSON.stringify([kind, target.source.toLowerCase(), kind === "name" ? "" : gender, target.base]);
    let id = ids.get(key);
    if (!id) {
      id = `${prefix}${batch.requests.length}`;
      ids.set(key, id);
      batch.requests.push({ id, name: target.source, kind, gender, base: target.base });
      batch.targets.set(id, []);
    }
    batch.targets.get(id)!.push(target);
  }
  return batch;
}

/** Первый шаг: имена и фамилии. Они станут основой для форм. */
export function baseNames(characters: NamedCharacter[]) {
  return nameBatch(characters.flatMap((char): WantedName[] => {
    const common = { alias: char.alias, gender: char.gender, base: "" };
    const first = char.first_name.trim();
    const last = char.last_name.trim();
    return [
      ...(first ? [{ ...common, field: "first", source: first, current: char.translated_first_name.trim(), kind: "name" } as const] : []),
      ...(last ? [{ ...common, field: "last", source: last, current: char.translated_last_name.trim(), kind: "surname" } as const] : [])
    ];
  }), "n");
}

/**
 * Второй шаг: формы строятся от готового написания. Заполненное поле важнее ответа модели,
 * потому что его выбрал пользователь, а `spelled` по `nameTargetKey` хранит ответы первого шага.
 */
export function formNames(characters: NamedCharacter[], spelled: Map<string, string>) {
  return nameBatch(characters.flatMap((char): WantedName[] => {
    const known = (field: NameField, source: string, current: string) =>
      current.trim() || spelled.get(nameTargetKey({ alias: char.alias, field, source: source.trim() })) || "";
    const base = joinName(known("first", char.first_name, char.translated_first_name), known("last", char.last_name, char.translated_last_name));
    const common = { alias: char.alias, gender: char.gender, base, kind: "form" as const };
    const taken = canonicalForms(char).map((form) => form.source);
    const isTaken = (source: string) => taken.some((name) => sameName(name, source));
    return char.forms
      .filter((form) => form.source.trim() && !isTaken(form.source))
      .map((form) => ({ ...common, field: "form" as const, source: form.source.trim(), current: form.translated.trim() }));
  }), "f");
}
