import { invoke } from "@tauri-apps/api/core";

export interface LanguageRule { language: string; rules: string }

export function normalizeLanguageName(language: string) {
  return language.trim().toLowerCase();
}

export function missingNameRuleLanguages(origins: { languages: string[] }[]) {
  return [...new Set(origins.flatMap((origin) => origin.languages).map(normalizeLanguageName))]
    .filter((language) => language && !["описание", "неизвестно"].includes(language)
      && !nameRules.items.some((rule) => rule.language === language && rule.rules.trim()));
}

// Одна библиотека у вкладки правил и перевода имён: правки видны сразу в обоих местах.
export const nameRules = $state({
  items: [] as LanguageRule[],
  saved: {} as Record<string, string>,
  removed: [] as string[],
  loading: false,
  saving: false
});

export const dirtyNameRuleLanguages = () => [...new Set([...nameRules.items
  .filter((rule) => rule.rules !== (nameRules.saved[rule.language] ?? ""))
  .map((rule) => rule.language), ...nameRules.removed])];

export function setNameRule(language: string, rules: string) {
  language = normalizeLanguageName(language);
  if (!language) return;
  nameRules.removed = nameRules.removed.filter((key) => key !== language);
  const own = nameRules.items.find((rule) => rule.language === language);
  if (own) own.rules = rules;
  else nameRules.items.push({ language, rules });
}

export function removeNameRule(language: string) {
  language = normalizeLanguageName(language);
  nameRules.items = nameRules.items.filter((rule) => rule.language !== language);
  if (Object.hasOwn(nameRules.saved, language) || nameRules.saving) {
    nameRules.removed = [...new Set([...nameRules.removed, language])];
  }
}

export function restoreNameRule(language: string) {
  language = normalizeLanguageName(language);
  if (Object.hasOwn(nameRules.saved, language)) setNameRule(language, nameRules.saved[language]);
}

export function renameNameRule(language: string, renamed: string) {
  language = normalizeLanguageName(language);
  renamed = normalizeLanguageName(renamed);
  if (!renamed || ["описание", "неизвестно"].includes(renamed)) throw new Error("Укажите язык происхождения или название вымышленного языка.");
  if (renamed === language) return;
  if (nameRules.items.some((rule) => rule.language === renamed)) throw new Error("Правила для этого языка уже существуют.");
  const rule = nameRules.items.find((rule) => rule.language === language);
  if (!rule) return;
  const text = rule.rules;
  removeNameRule(language);
  setNameRule(renamed, text);
}

let pendingLoad: Promise<void> | null = null;

export async function loadNameRules() {
  if (pendingLoad) return pendingLoad;
  nameRules.loading = true;
  pendingLoad = (async () => {
    try {
      const saved = await invoke<LanguageRule[]>("load_name_rules");
      const dirty = new Set(dirtyNameRuleLanguages());
      for (const rule of saved) {
        if (!dirty.has(rule.language)) setNameRule(rule.language, rule.rules);
        nameRules.saved[rule.language] = rule.rules;
      }
    } finally {
      nameRules.loading = false;
      pendingLoad = null;
    }
  })();
  return pendingLoad;
}

export async function saveNameRules() {
  if (nameRules.saving) throw new Error("Дождитесь сохранения правил.");
  const wanted = new Set(dirtyNameRuleLanguages());
  const rules = nameRules.items.filter((rule) => wanted.has(rule.language)).map((rule) => ({ ...rule }));
  const removed = nameRules.removed.filter((language) => wanted.has(language));
  if (rules.length === 0 && removed.length === 0) return;
  if (rules.some((rule) => !rule.rules.trim())) throw new Error("Заполните текст правил перед сохранением.");
  nameRules.saving = true;
  try {
    await invoke("save_name_rules", { rules, removed });
    // Если текст успели изменить во время записи, новая правка остаётся несохранённой.
    for (const rule of rules) nameRules.saved[rule.language] = rule.rules;
    for (const language of removed) delete nameRules.saved[language];
    nameRules.removed = nameRules.removed.filter((language) => !removed.includes(language));
  } finally { nameRules.saving = false; }
}
