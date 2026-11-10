/** Глубина чтения для описаний моделью, как `ReadingDepth` в Rust. Части по ней считает Rust. */
export interface ReadingDepth {
  min: number;
  /** `null`: без предела. */
  max: number | null;
  percent: number;
}

export const DEFAULT_READING_DEPTH: ReadingDepth = { min: 5, max: 50, percent: 30 };

/** Защищает от пустых полей, дробного числа частей и неверных границ. Максимум `null` или не число: без предела. */
export function normalizeReadingDepth(depth: ReadingDepth): ReadingDepth {
  const min = Number.isFinite(depth.min) ? Math.max(1, Math.floor(depth.min)) : 1;
  const max = depth.max !== null && Number.isFinite(depth.max) ? Math.max(min, Math.floor(depth.max)) : null;
  const percent = Number.isFinite(depth.percent) ? Math.min(100, Math.max(1, Math.round(depth.percent))) : 100;
  return { min, max, percent };
}

/** Сколько частей из `total` прочитает модель при этой глубине: как `ReadingDepth::parts_for` в Rust. */
export function partsFor(depth: ReadingDepth, total: number): number {
  const { min, max, percent } = normalizeReadingDepth(depth);
  if (total <= min) return Math.max(total, 0);
  const wanted = Math.max(Math.ceil((total * percent) / 100), min);
  const capped = max === null ? wanted : Math.min(wanted, Math.max(max, min));
  return Math.min(capped, total);
}
