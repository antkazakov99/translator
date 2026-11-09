// Примеры текста и оценка шрифта для раздела подмены шрифтов.

import type { FontCoverageDto, GameFontDto } from "./bindings";

export type FontCoverage = FontCoverageDto;
export type GameFont = GameFontDto;

export type FontStatusKind = "ok" | "letters" | "marks" | "symbols" | "unknown";
export type FontStatus = { kind: FontStatusKind; label: string };
export type SampleSegment = { text: string; missing: boolean };

export const ORIGINAL_SAMPLE = {
  label: "Оригинал",
  text: "The quick brown fox jumps over the lazy dog. 0123456789"
};

// Язык перевода пока один (TL_LANG в engines/renpy): новый язык добавится сюда
const LANGUAGE_SAMPLES = {
  russian: {
    label: "Русский",
    text: "Съешь же ещё этих мягких французских булок, да выпей чаю. ЁЖИК — «Привет!» 0123…",
    letters: "АБВГДЕЁЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯабвгдеёжзийклмнопрстуфхцчшщъыьэюя",
    marks: "«»—…",
    hasLetters: "Есть кириллица",
    noLetters: "Нет кириллицы"
  }
};

export const TARGET_SAMPLE = LANGUAGE_SAMPLES.russian;

const LATIN_LETTERS = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/** Движок показывает вместо пропавшего символа пустой глиф, а браузер взял бы другой шрифт. */
export function sampleSegments(text: string, missing: string): SampleSegment[] {
  const absent = new Set(missing);
  const segments: SampleSegment[] = [];
  for (const char of text) {
    const isMissing = absent.has(char);
    const last = segments[segments.length - 1];
    if (last && last.missing === isMissing) {
      last.text += char;
    } else {
      segments.push({ text: char, missing: isMissing });
    }
  }
  return segments;
}

export function fontStatus(coverage: FontCoverage): FontStatus {
  if (coverage.unreadable) return { kind: "unknown", label: "Файл не прочитан" };
  const missing = [...coverage.missingTarget];
  const letters = missing.filter((char) => !TARGET_SAMPLE.marks.includes(char));
  const marks = missing.filter((char) => TARGET_SAMPLE.marks.includes(char));
  if (letters.length > 0) {
    // Шрифт без латиницы и языка перевода это обычно значки, и заменять его нельзя
    const hasLatin = [...LATIN_LETTERS].some((char) => !coverage.missingOriginal.includes(char));
    if (!hasLatin) return { kind: "symbols", label: "Без букв: возможно, значки" };
    return {
      kind: "letters",
      label: letters.length >= [...TARGET_SAMPLE.letters].length
        ? TARGET_SAMPLE.noLetters
        : `Нет букв: ${letters.slice(0, 8).join("")}${letters.length > 8 ? "…" : ""}${marks.length > 0 ? `; знаков: ${marks.join(" ")}` : ""}`
    };
  }
  if (missing.length > 0) return { kind: "marks", label: `Нет знаков: ${missing.join(" ")}` };
  return { kind: "ok", label: TARGET_SAMPLE.hasLetters };
}

/** Шрифт, которому нужна замена, и которого она ещё не получила. */
export function needsReplacement(font: GameFont): boolean {
  if (font.replacement || font.locked) return false;
  const kind = fontStatus(font.coverage).kind;
  return kind === "letters" || kind === "marks";
}

/** Сначала то, что требует решения. */
export function fontOrder(a: GameFont, b: GameFont): number {
  const rank = (font: GameFont) => (needsReplacement(font) ? 0 : 1);
  return rank(a) - rank(b);
}
