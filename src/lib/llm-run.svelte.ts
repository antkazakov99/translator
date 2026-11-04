/**
 * Действие через модель, которое идёт сейчас. Остановка в Rust общая на все запросы, поэтому
 * запуски идут по одному: новое действие начинается с `beginLlmRun`, а его кнопки и настройки
 * отключаются по `llmRun.active`.
 */
export interface LlmRequestProgress {
  phase: "analysis" | "translation" | "validation" | "rules" | "completed";
  batch: number;
  total_batches: number;
  completed: number;
  total: number;
  attempt: number;
  names?: string[];
}

export const llmRun = $state({ active: false, note: "", startedAt: 0, requestStartedAt: 0, progress: null as LlmRequestProgress | null });

export function setLlmProgress(progress: LlmRequestProgress) {
  if (!llmRun.active) return;
  const previous = llmRun.progress;
  if (progress.phase !== "completed" && (!previous || previous.phase !== progress.phase || previous.batch !== progress.batch || previous.attempt !== progress.attempt)) {
    llmRun.requestStartedAt = Date.now();
  }
  llmRun.progress = progress;
}

/** Занимает модель. `false` значит, что идёт другой запуск и начинать нельзя. */
export function beginLlmRun(note: string) {
  if (llmRun.active) return false;
  llmRun.active = true;
  llmRun.note = note;
  llmRun.startedAt = Date.now();
  llmRun.requestStartedAt = llmRun.startedAt;
  llmRun.progress = null;
  return true;
}

export function endLlmRun() {
  llmRun.active = false;
  llmRun.note = "";
  llmRun.progress = null;
}
