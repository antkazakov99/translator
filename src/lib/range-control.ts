export interface RangeState {
  max: number;
  value: number;
}

/** Максимум задаётся до значения: браузер мог ограничить прежнюю позицию старой шкалой. */
export function syncRange(input: HTMLInputElement, initial: RangeState) {
  function update({ max, value }: RangeState) {
    const upper = Number.isFinite(max) ? Math.max(0, max) : 0;
    input.min = "0";
    input.step = "1";
    input.max = String(upper);
    input.value = String(Number.isFinite(value) ? Math.min(upper, Math.max(0, value)) : 0);
  }
  update(initial);
  return { update };
}
