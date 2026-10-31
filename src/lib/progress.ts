// Процент для показа: 0 только без перевода, 100 только когда переведено всё.
// Обычное округление прячет одну строку большого файла и выдаёт почти готовый файл за готовый.
export function progressPercent(done: number, total: number): number {
  if (total <= 0 || done <= 0) return 0;
  if (done >= total) return 100;
  return Math.min(99, Math.max(1, Math.round((done / total) * 100)));
}

export function progressLabel(done: number, total: number): string {
  if (done > 0 && done / total < 0.01) return '<1%';
  return `${progressPercent(done, total)}%`;
}
