import type { GameStep, RunProgress_Deserialize as RunProgress, StepResultDto, StepWorkDto } from "./bindings";

/** Шаги автоперевода в порядке выполнения: тот же порядок держит Rust (`GameStep::ALL`). */
export const gameSteps: GameStep[] = ["analysis", "names", "glossary", "entries", "scenes", "lines", "review"];

export const stepTitles: Record<GameStep, string> = {
  analysis: "Разбор сцен",
  names: "Перевод имён",
  glossary: "Переводы глоссария",
  entries: "Описания персонажей и терминов",
  scenes: "Описания сцен",
  lines: "Перевод реплик",
  review: "Вычитка",
};

/** Что делает шаг: подпись в панели, пока не посчитано, сколько ему осталось. */
export const stepHints: Record<GameStep, string> = {
  analysis: "Модель находит в сценах персонажей и термины и пишет их в глоссарий",
  names: "Написание имён и фамилий персонажей, потом форм. Раньше остальных переводов: термины и подписи с именем берут готовое написание",
  glossary: "Перевод терминов, подписей говорящих и значения токенов",
  entries: "Описания персонажей и терминов по их сценам",
  scenes: "Названия и описания сцен",
  lines: "Перевод пустых реплик",
  review: "Вычитка переведённых реплик, которые модель ещё не вычитывала",
};

/** Отмеченные шаги в порядке выполнения. */
export function orderedSteps(chosen: GameStep[]): GameStep[] {
  return gameSteps.filter((step) => chosen.includes(step));
}

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

/** Строка общей полосы: «Шаг 2 из 4 · Перевод терминов: запрос 3 из 5 · записано: 2 · в пакете: Rusty Anchor». */
export function chainText(progress: RunProgress, stopping: boolean) {
  if (stopping) return "Выполнение: останавливается после текущего запроса";
  const stage = progress.stage;
  const head = stage ? `Шаг ${stage.index} из ${stage.count} · ${stage.title}` : "Выполнение";
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

/**
 * Шаги до отбора имён и после него одним списком. Шаг имён проходит дважды, до отбора и после него:
 * записанное складывается.
 */
export function mergeFinished(earlier: StepResultDto[], later: StepResultDto[]): StepResultDto[] {
  const merged = earlier.map((item) => ({ ...item }));
  for (const item of later) {
    const known = merged.find((done) => done.step === item.step);
    if (known) known.written += item.written;
    else merged.push({ ...item });
  }
  return merged;
}

/** Итог: «Готово: Разбор сцен · 3, Перевод реплик · 120». */
export function finishedText(finished: StepResultDto[], stopped: boolean) {
  const done = finished.map((item) => `${stepTitles[item.step]} · ${formatCount(item.written)}`).join(", ");
  if (stopped) return done ? `Остановлено. Пройдены шаги: ${done}` : "Остановлено до конца первого шага";
  return done ? `Готово: ${done}` : "Делать было нечего";
}
