/** Глубина чтения для описаний моделью, как `ReadingDepth` в Rust. Части по ней считает Rust. */
export interface ReadingDepth {
  min: number;
  max: number;
  percent: number;
}

export const DEFAULT_READING_DEPTH: ReadingDepth = { min: 5, max: 50, percent: 30 };

/** Защищает от пустых полей, дробного числа частей и неверных границ. */
export function normalizeReadingDepth(depth: ReadingDepth): ReadingDepth {
  const min = Number.isFinite(depth.min) ? Math.max(1, Math.floor(depth.min)) : 1;
  const max = Number.isFinite(depth.max) ? Math.max(min, Math.floor(depth.max)) : min;
  const percent = Number.isFinite(depth.percent) ? Math.min(100, Math.max(1, Math.round(depth.percent))) : 100;
  return { min, max, percent };
}

/** Сколько частей из `total` прочитает модель при этой глубине: как `ReadingDepth::parts_for` в Rust. */
export function partsFor(depth: ReadingDepth, total: number): number {
  const { min, max, percent } = normalizeReadingDepth(depth);
  if (total <= min) return Math.max(total, 0);
  const wanted = Math.ceil((total * percent) / 100);
  return Math.min(Math.min(Math.max(wanted, min), Math.max(max, min)), total);
}
