import type { ProviderKind } from "./bindings";

/** Виды провайдеров для окна: подпись, адрес по умолчанию и где взять ключ. */
export const PROVIDER_KINDS: { id: ProviderKind; label: string; url: string; keyHint: string }[] = [
  { id: "local", label: "Локальный сервер", url: "http://127.0.0.1:1234/v1", keyHint: "" },
  { id: "openai", label: "ChatGPT", url: "https://api.openai.com/v1", keyHint: "Ключ создаётся на platform.openai.com/api-keys" },
  { id: "claude", label: "Claude", url: "https://api.anthropic.com", keyHint: "Ключ создаётся на platform.claude.com/settings/keys" },
  { id: "gemini", label: "Gemini", url: "https://generativelanguage.googleapis.com/v1beta/openai", keyHint: "Ключ создаётся на aistudio.google.com/apikey" },
  { id: "compatible", label: "Другой сервер", url: "", keyHint: "" },
];

export const kindOf = (id: ProviderKind) => PROVIDER_KINDS.find((kind) => kind.id === id) ?? PROVIDER_KINDS[PROVIDER_KINDS.length - 1];

/** Имя нового провайдера: подпись вида, с номером, если такое имя уже занято. */
export function newProviderName(kind: ProviderKind, taken: string[]) {
  const label = kindOf(kind).label;
  if (!taken.includes(label)) return label;
  let number = 2;
  while (taken.includes(`${label} ${number}`)) number++;
  return `${label} ${number}`;
}

const NOT_TEXT_MODEL = /embed|\btts\b|whisper|transcribe|dall-e|image|\bsora|\bveo|moderation|realtime|audio|speech|lyria|nano-banana|robotics|turbo-instruct|babbage|davinci/i;

/** Модели, которые пишут текст, по имени: числа сравниваются как числа, поэтому 4.1 идёт раньше 10. */
export function textModels(models: string[]) {
  return models
    .filter((model) => !NOT_TEXT_MODEL.test(model))
    .sort((a, b) => a.localeCompare(b, "en", { numeric: true, sensitivity: "base" }));
}

/** Целое из поля ввода. Пусто: значение из кода. `undefined`: в поле не число. */
export function optionalInteger(text: string, min: number): number | null | undefined {
  const value = text.trim();
  if (value === "") return null;
  if (!/^\d+$/.test(value)) return undefined;
  const number = Number(value);
  return number >= min && number <= 0xffffffff ? number : undefined;
}

/** Температура из поля ввода, от 0 до 2, запятая тоже годится. */
export function optionalTemperature(text: string): number | null | undefined {
  const value = text.trim().replace(",", ".");
  if (value === "") return null;
  if (!/^\d+(\.\d+)?$/.test(value)) return undefined;
  const number = Number(value);
  return number <= 2 ? number : undefined;
}
