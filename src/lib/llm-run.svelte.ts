/**
 * Действие через модель, которое идёт сейчас. Остановка в Rust общая на все запросы, поэтому
 * запуски идут по одному: новое действие начинается с `beginLlmRun`, а его кнопки и настройки
 * отключаются по `llmRun.active`.
 */
export const llmRun = $state({ active: false, note: "" });

/** Занимает модель. `false` значит, что идёт другой запуск и начинать нельзя. */
export function beginLlmRun(note: string) {
  if (llmRun.active) return false;
  llmRun.active = true;
  llmRun.note = note;
  return true;
}

export function endLlmRun() {
  llmRun.active = false;
  llmRun.note = "";
}
