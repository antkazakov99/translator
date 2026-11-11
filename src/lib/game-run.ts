import type { GameStep, RunProgress_Deserialize as RunProgress, StepResultDto, StepWorkDto } from "./bindings";

/** Шаги «Вся игра» в порядке выполнения: тот же порядок держит Rust (`GameStep::ALL`). */
export const gameSteps: GameStep[] = ["analysis", "glossary", "entries", "scenes", "lines", "review"];

export const stepTitles: Record<GameStep, string> = {
  analysis: "Разбор сцен",
  glossary: "Переводы глоссария",
  entries: "Описания персонажей и терминов",
  scenes: "Описания сцен",
  lines: "Перевод реплик",
  review: "Вычитка",
};

const numbers = new Intl.NumberFormat("ru-RU");

/** «12 300». */
export function formatCount(value: number) {
  return numbers.format(value);
}

/** Русская форма слова по числу: `plural(3, ["запрос", "запроса", "запросов"])`. */
export function plural(value: number, forms: [string, string, string]) {
  const tens = value % 100;
  const ones = value % 10;
  if (tens >= 11 && tens <= 14) return forms[2];
  if (ones === 1) return forms[0];
  if (ones >= 2 && ones <= 4) return forms[1];
  return forms[2];
}

/** «≈ 1 234 запроса». */
export function requestsLabel(value: number) {
  return `≈ ${formatCount(value)} ${plural(value, ["запрос", "запроса", "запросов"])}`;
}

/** Что осталось шагу: «сцен без описания: 140». У глоссария только то, что есть: «терминов без перевода: 5 · подписей без перевода: 2». */
export function leftLabel(work: StepWorkDto) {
  const shown = work.left.filter((item) => item.count > 0);
  if (shown.length === 0) return "Делать нечего";
  return shown.map((item) => `${item.label}: ${formatCount(item.count)}`).join(" · ");
}

/**
 * Шаги, которые дают работу шагу позже в очереди: разбор находит персонажей и термины для переводов глоссария
 * и описаний, перевод даёт реплики вычитке. Шаг без работы сейчас доступен, если отмечен такой шаг раньше.
 */
export const feeders: Partial<Record<GameStep, GameStep[]>> = {
  glossary: ["analysis"],
  entries: ["analysis"],
  review: ["lines"],
};

/** Отмеченный раньше шаг, после которого у шага появится работа. `null`: работа есть сейчас или взяться ей неоткуда. */
export function fedBy(step: GameStep, chosen: GameStep[], work: StepWorkDto[]): GameStep | null {
  if (work.some((item) => item.step === step && !item.empty)) return null;
  const ready = availableSteps(chosen, work);
  return (feeders[step] ?? []).find((feeder) => chosen.includes(feeder) && ready.has(feeder)) ?? null;
}

/** Шаги, которые можно отметить: есть работа сейчас или её даст отмеченный раньше доступный шаг. */
export function availableSteps(chosen: GameStep[], work: StepWorkDto[]): Set<GameStep> {
  const ready = new Set<GameStep>();
  for (const step of gameSteps) {
    const item = work.find((known) => known.step === step);
    if (!item) continue;
    const later = (feeders[step] ?? []).some((feeder) => chosen.includes(feeder) && ready.has(feeder));
    if (!item.empty || later) ready.add(step);
  }
  return ready;
}

/** Отмеченные по умолчанию: шаги, где есть что делать сейчас или после шагов раньше. */
export function defaultSteps(work: StepWorkDto[]): GameStep[] {
  return [...availableSteps(gameSteps, work)];
}

/** Шаги к запуску: отмеченные и доступные, в порядке выполнения. */
export function stepsToRun(chosen: GameStep[], work: StepWorkDto[]): GameStep[] {
  const ready = availableSteps(chosen, work);
  return gameSteps.filter((step) => chosen.includes(step) && ready.has(step));
}

/** Строка общей полосы: «Шаг 2 из 4 · Перевод терминов: запрос 3 из 5 · записано: 2 · в пакете: Rusty Anchor». */
export function chainText(progress: RunProgress, stopping: boolean) {
  if (stopping) return "Вся игра: останавливается после текущего запроса";
  const stage = progress.stage;
  const head = stage ? `Шаг ${stage.index} из ${stage.count} · ${stage.title}` : "Вся игра";
  if (progress.total === 0) return `${head}: подготовка`;
  const counts = `${head}: запрос ${Math.min(progress.done + 1, progress.total)} из ${progress.total} · записано: ${progress.written}`;
  return progress.message ? `${counts} · ${progress.message}` : counts;
}

/** Чем кончился шаг в последнем запуске цепочки. */
export type StepOutcome =
  | { kind: "done"; written: number }
  | { kind: "error"; message: string }
  | { kind: "stopped" }
  /** Цепочка встала на отборе имён или оборвалась раньше. */
  | { kind: "waiting" }
  /** Шаг не отмечали или делать было нечего. */
  | { kind: "skipped" };

/**
 * Итог последнего запуска по каждому шагу: пройденные с числом записанного, шаг с ошибкой или остановкой,
 * отмеченные, до которых очередь не дошла, и пропущенные.
 */
export function stepOutcomes(
  requested: GameStep[],
  result: { finished: StepResultDto[]; stopped: boolean; error?: string | null },
): Record<GameStep, StepOutcome> {
  const outcomes = {} as Record<GameStep, StepOutcome>;
  let broken = false;
  for (const step of gameSteps) {
    const done = result.finished.find((item) => item.step === step);
    if (done) outcomes[step] = { kind: "done", written: done.written };
    else if (!requested.includes(step)) outcomes[step] = { kind: "skipped" };
    else if (!broken && result.error) {
      broken = true;
      outcomes[step] = { kind: "error", message: result.error };
    } else if (!broken && result.stopped) {
      broken = true;
      outcomes[step] = { kind: "stopped" };
    } else outcomes[step] = { kind: "waiting" };
  }
  return outcomes;
}

/** Итог: «Готово: Разбор сцен · 3, Перевод реплик · 120». */
export function finishedText(finished: StepResultDto[], stopped: boolean) {
  const done = finished.map((item) => `${stepTitles[item.step]} · ${formatCount(item.written)}`).join(", ");
  if (stopped) return done ? `Остановлено. Пройдены шаги: ${done}` : "Остановлено до конца первого шага";
  return done ? `Готово: ${done}` : "Делать было нечего";
}
