export interface NameVariant { translated: string; description: string }

/** Короткое пояснение вместо длинного отчёта модели. */
const shortDescription = (text: string) => text.replace(/[*_`#]/g, "").replace(/\s+/g, " ").trim().slice(0, 180);

export function nameOptions(suggestion: { translated: string; variants?: NameVariant[]; glossary_variants?: NameVariant[] }, current: string): NameVariant[] {
  const options: NameVariant[] = [];
  const add = (translated: string, description: string) => {
    translated = translated.trim();
    if (!translated) return;
    const existing = options.find((option) => option.translated === translated);
    if (existing) {
      if (translated === current.trim() && description.trim()) existing.description = shortDescription(description);
      return;
    }
    options.push({ translated, description: shortDescription(description) });
  };
  if (current.trim()) add(current, suggestion.glossary_variants?.find((variant) => variant.translated.trim() === current.trim())?.description || "Сохранённое написание; пояснение модели пока отсутствует.");
  if (suggestion.variants?.length) {
    for (const variant of suggestion.variants.slice(0, 3)) add(variant.translated, variant.description || "Вариант по предполагаемому произношению.");
  } else if (suggestion.translated.trim() !== current.trim()) add(suggestion.translated, "Вариант по предполагаемому произношению.");
  return options;
}

export const nameCheckNotice = (issues: string[]) => issues.some((issue) => /не удалось|не удался|пропустила проверку|проверка остановлена|отдельная проверка не выполнялась/i.test(issue)) ? "Проверка не завершена." : "";

/** Рекомендация всегда указывает на существующую радиокнопку. */
export function modelNameChoice(suggestion: { translated: string; variants?: NameVariant[] }, options: NameVariant[]) {
  const candidates = [...(suggestion.variants ?? []).map((variant) => variant.translated), suggestion.translated];
  for (const candidate of candidates) {
    const option = options.find((option) => option.translated === candidate.trim());
    if (option) return option.translated;
  }
  return options[0]?.translated ?? "";
}
