export interface CharacterChunks {
  total: number;
  ready: number;
}

/** Сколько частей войдёт в запуск: готовые входят всегда, а больше, чем есть у персонажа, не прочитать. */
export function partsToRead({ total, ready }: CharacterChunks, wanted: number) {
  return Math.min(Math.max(wanted, ready), total);
}
