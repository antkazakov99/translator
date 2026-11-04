// Примеры текста и оценка шрифта для раздела подмены шрифтов.

export type FontCoverage = {
  unreadable: boolean;
  missing_original: string;
  missing_target: string;
};

export type FontReplacement = {
  source_name: string;
  coverage: FontCoverage;
};

export type GameFont = {
  name: string;
  origin: "game" | "archive" | "engine" | "translation";
  references: number;
  coverage: FontCoverage;
  locked: string | null;
  replacement: FontReplacement | null;
};

export type FontStatusKind = "ok" | "letters" | "marks" | "symbols" | "unknown";
export type FontStatus = { kind: FontStatusKind; label: string };
export type SampleSegment = { text: string; missing: boolean };

export const ORIGINAL_SAMPLE = {
  label: "Оригинал",
  text: "The quick brown fox jumps over the lazy dog. 0123456789"
};

// Язык проекта пока один (TL_LANG в lib.rs): новый язык добавится сюда
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
  const missing = [...coverage.missing_target];
  const letters = missing.filter((char) => !TARGET_SAMPLE.marks.includes(char));
  const marks = missing.filter((char) => TARGET_SAMPLE.marks.includes(char));
  if (letters.length > 0) {
    // Шрифт без латиницы и языка перевода это обычно значки, и заменять его нельзя
    const hasLatin = [...LATIN_LETTERS].some((char) => !coverage.missing_original.includes(char));
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
  if (font.replacement || font.origin === "engine") return false;
  const kind = fontStatus(font.coverage).kind;
  return kind === "letters" || kind === "marks";
}

/** Сначала то, что требует решения, а шрифты движка в конце. */
export function fontOrder(a: GameFont, b: GameFont): number {
  const rank = (font: GameFont) => (needsReplacement(font) ? 0 : font.origin === "engine" ? 2 : 1);
  return rank(a) - rank(b);
}

export function originLabel(origin: GameFont["origin"]): string {
  switch (origin) {
    case "archive":
      return "Архив игры";
    case "engine":
      return "Шрифт движка";
    case "translation":
      return "Папка перевода";
    default:
      return "Папка игры";
  }
}
