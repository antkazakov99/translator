/**
 * Запуск модели, который идёт сейчас. Rust пускает один запуск за раз во всём приложении (`RunSlot`), поэтому
 * и в окне он один: его показывает общая полоса под шапкой, где бы он ни начался.
 */
export interface RunSource {
  /** «Перевод: запрос 2 из 7 · записано: 12». */
  readonly text: string;
  readonly stopping: boolean;
  stop(onError: (message: string) => void): void | Promise<void>;
}

class ActiveRun {
  /** Источник не проксируется: его поля уже реактивны, а сравнивать нужно сам объект. */
  current = $state.raw<RunSource | null>(null);
}

export const activeRun = new ActiveRun();

/** Отмечает запуск как текущий. Возвращает снятие отметки: оно не трогает запуск, начатый позже. */
export function trackRun(source: RunSource): () => void {
  activeRun.current = source;
  return () => {
    if (activeRun.current === source) activeRun.current = null;
  };
}
