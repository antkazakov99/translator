import type {
  CharacterDto_Serialize,
  FormDto_Serialize,
  Gender,
  GlossaryDto_Serialize,
  SpeakerDto_Serialize,
  SpeakerSuggestionDto_Serialize,
  TermCategory,
  TermDto_Serialize,
  TermSuggestionDto_Serialize,
  TokenDto_Serialize,
  TokenSuggestionDto_Serialize,
} from "./bindings";
import type { SuggestRun } from "./suggest-run.svelte";

export type Glossary = GlossaryDto_Serialize;
export type Character = CharacterDto_Serialize;
export type Speaker = SpeakerDto_Serialize;
export type Term = TermDto_Serialize;
export type Token = TokenDto_Serialize;
export type NameForm = FormDto_Serialize;
export type TermSuggestion = TermSuggestionDto_Serialize;
export type SpeakerSuggestion = SpeakerSuggestionDto_Serialize;
export type TokenSuggestion = TokenSuggestionDto_Serialize;

/** Запуски предложений модели по разделам глоссария. */
export type GlossaryRuns = {
  terms: SuggestRun<TermSuggestion>;
  speakers: SuggestRun<SpeakerSuggestion>;
  tokens: SuggestRun<TokenSuggestion>;
};

/** Идёт ли запуск или открыт отбор: новый запуск тогда недоступен. */
export function runsBusy(runs: GlossaryRuns) {
  return Object.values(runs).some((run) => run.active || run.review !== null);
}

/** Имя и фамилия через пробел, как подпись карточки. */
export function characterName(character: Pick<Character, "firstName" | "lastName">) {
  return [character.firstName, character.lastName].filter((part) => part?.trim()).join(" ") || "Без имени";
}

/** Перевод имени и фамилии, если он есть. */
export function characterTranslation(character: Pick<Character, "translatedFirstName" | "translatedLastName">) {
  return [character.translatedFirstName, character.translatedLastName].filter((part) => part?.trim()).join(" ");
}

/** Подпись говорящего: подпись из игры, иначе ключ без вида. */
export function speakerName(speaker: Pick<Speaker, "key" | "label">) {
  return speaker.label?.trim() || speaker.key.replace(/^[a-z]+:/, "");
}

/**
 * Код говорящего в скрипте, если он что-то добавляет к имени. Виды ключа задаёт движок: у Ren'Py `code`
 * это определение `Character`, а `label` подпись строкой, где ключ повторяет саму подпись.
 */
export function speakerCode(speaker: Pick<Speaker, "key" | "label">): string | null {
  const split = speaker.key.indexOf(":");
  if (split < 0 || speaker.key.slice(0, split) !== "code") return null;
  const code = speaker.key.slice(split + 1);
  return code === speakerName(speaker) ? null : code;
}

export const genders: { value: Gender | null; label: string }[] = [
  { value: null, label: "Не знаю" },
  { value: "male", label: "Мужской" },
  { value: "female", label: "Женский" },
];

export const categories: { value: TermCategory; label: string }[] = [
  { value: "general", label: "Общее" },
  { value: "location", label: "Место" },
  { value: "organization", label: "Организация" },
  { value: "title", label: "Звание" },
  { value: "item", label: "Предмет" },
  { value: "event", label: "Событие" },
];

export function categoryLabel(category: TermCategory) {
  return categories.find((item) => item.value === category)?.label ?? category;
}

/** Поиск без учёта регистра по нескольким полям. */
export function matches(query: string, ...fields: (string | null | undefined)[]) {
  const needle = query.trim().toLowerCase();
  return !needle || fields.some((field) => field?.toLowerCase().includes(needle));
}
