export interface CharacterChunks {
  total: number;
  ready: number;
}

export interface ReadingDepth {
  min: number;
  max: number;
  percent: number;
}

export const DEFAULT_READING_DEPTH: ReadingDepth = { min: 5, max: 50, percent: 30 };

/** Защищает расчёт от пустых полей, дробного числа частей и неверных границ. */
export function normalizeReadingDepth(depth: ReadingDepth): ReadingDepth {
  const min = Number.isFinite(depth.min) ? Math.max(1, Math.floor(depth.min)) : 1;
  const max = Number.isFinite(depth.max) ? Math.max(min, Math.floor(depth.max)) : min;
  const percent = Number.isFinite(depth.percent) ? Math.min(100, Math.max(1, depth.percent)) : 100;
  return { min, max, percent };
}

/** Процент считается от всех частей записи; готовые заметки сохраняются сверх лимита. */
export function partsForDepth(chunks: CharacterChunks, depth: ReadingDepth) {
  const { min, max, percent } = normalizeReadingDepth(depth);
  return partsToRead(chunks, Math.max(min, Math.min(max, Math.ceil(chunks.total * percent / 100))));
}

/** Сколько частей войдёт в запуск: готовые входят всегда, а больше, чем есть у персонажа, не прочитать. */
export function partsToRead({ total, ready }: CharacterChunks, wanted: number) {
  return Math.min(Math.max(wanted, ready), total);
}

/** Запуск не добавит заметок: всё, что он взял бы, уже прочитано. */
export function nothingToRead(chunks: CharacterChunks, wanted: number) {
  return partsToRead(chunks, wanted) <= chunks.ready;
}
