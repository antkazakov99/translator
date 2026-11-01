export interface CharacterChunks {
  total: number;
  ready: number;
}

/** Сколько запросов займёт персонаж: готовые части повторно не читаются, а описание пишется всегда. */
export function requestsFor({ ready }: CharacterChunks, parts: number) {
  // Одна новая часть читается тем же запросом, что пишет описание
  if (parts === 1 && ready === 0) return 1;
  return Math.max(parts - ready, 0) + 1;
}

// Готовые части входят в запуск всегда, как и на ползунке отдельного персонажа
const fewestParts = ({ ready }: CharacterChunks) => Math.max(ready, 1);

export function requestRange(characters: CharacterChunks[]) {
  return {
    min: characters.reduce((sum, c) => sum + requestsFor(c, fewestParts(c)), 0),
    max: characters.reduce((sum, c) => sum + requestsFor(c, c.total), 0)
  };
}

/**
 * Делит `budget` запросов между персонажами и возвращает число частей каждого. Очередная часть
 * достаётся тому, у кого прочитана меньшая доля сцен, поэтому большие роли получают больше запросов.
 */
export function distributeRequests(characters: CharacterChunks[], budget: number) {
  const parts = characters.map(fewestParts);
  let left = budget - characters.reduce((sum, c, i) => sum + requestsFor(c, parts[i]), 0);
  for (;;) {
    let next = -1;
    let nextCost = 0;
    characters.forEach((c, i) => {
      const cost = requestsFor(c, parts[i] + 1) - requestsFor(c, parts[i]);
      if (parts[i] >= c.total || cost > left) return;
      if (next < 0 || parts[i] / c.total < parts[next] / characters[next].total) {
        next = i;
        nextCost = cost;
      }
    });
    if (next < 0) return parts;
    parts[next] += 1;
    left -= nextCost;
  }
}
