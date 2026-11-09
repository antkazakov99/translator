/** Номера строк (с нуля), где есть запрос без учёта регистра. Пустой запрос ничего не находит. */
export function matchingLines(lines: string[], query: string): number[] {
  const needle = query.trim().toLocaleLowerCase();
  if (!needle) return [];
  const found: number[] = [];
  lines.forEach((line, index) => {
    if (line.toLocaleLowerCase().includes(needle)) found.push(index);
  });
  return found;
}
