export type LlmServer = "local" | "external";
export type LlmAction = "descriptions" | "names" | "terms" | "termSearch" | "tokens";

export const LLM_ACTIONS: { id: LlmAction; label: string; sends: string; gameText: boolean }[] = [
  { id: "descriptions", label: "Описания персонажей", sends: "сцены с репликами", gameText: true },
  { id: "names", label: "Имена", sends: "только список имён", gameText: false },
  { id: "terms", label: "Перевод терминов", sends: "только термины", gameText: false },
  { id: "termSearch", label: "Поиск терминов", sends: "названия и примеры реплик", gameText: true },
  { id: "tokens", label: "Токены", sends: "реплики с токенами", gameText: true }
];

export interface LlmActionSettings {
  server: LlmServer;
  model: string;
}

export interface LlmSettings {
  localUrl: string;
  externalUrl: string;
  externalKey: string;
  actions: Record<LlmAction, LlmActionSettings>;
}

export interface LlmTarget {
  apiUrl: string;
  model: string;
  apiKey: string | null;
}

const SETTINGS_KEY = "translator.llm_settings";
const DEFAULT_URL = "http://127.0.0.1:1234/v1";
const DEFAULT_MODEL = "mistral-small";

/** Настройки из сохранённого JSON. Без него берутся значения прежних версий из `legacy`. */
export function parseLlmSettings(saved: string | null, legacy: (key: string) => string | null): LlmSettings {
  let stored: Partial<LlmSettings> = {};
  try {
    stored = saved ? JSON.parse(saved) : {};
  } catch {
    // Повреждённые настройки заменяются обычными
  }
  const localModel = legacy("translator.llm_model")?.trim() || DEFAULT_MODEL;
  const oldNamesUrl = legacy("translator.llm_names_api_url")?.trim() ?? "";
  const oldNamesModel = legacy("translator.llm_names_model")?.trim() ?? "";
  const fallback = (id: LlmAction): LlmActionSettings =>
    id === "names" && oldNamesUrl
      ? { server: "external", model: oldNamesModel }
      : { server: "local", model: localModel };
  const actions = Object.fromEntries(
    LLM_ACTIONS.map(({ id }) => {
      const action = stored.actions?.[id];
      return [id, action ? { server: action.server === "external" ? "external" : "local", model: action.model ?? "" } : fallback(id)];
    })
  ) as Record<LlmAction, LlmActionSettings>;
  return {
    localUrl: stored.localUrl ?? legacy("translator.llm_api_url") ?? DEFAULT_URL,
    externalUrl: stored.externalUrl ?? oldNamesUrl,
    externalKey: stored.externalKey ?? legacy("translator.llm_names_api_key") ?? "",
    actions
  };
}

export function loadLlmSettings() {
  const read = (key: string) => {
    try {
      return localStorage.getItem(key);
    } catch {
      return null;
    }
  };
  return parseLlmSettings(read(SETTINGS_KEY), read);
}

export function saveLlmSettings(settings: LlmSettings) {
  try {
    localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings));
  } catch {
    // Без хранилища настройки живут до закрытия приложения
  }
}

/** Куда отправить запрос действия. Ключ уходит только на внешний сервер. */
export function targetFor(settings: LlmSettings, action: LlmAction): LlmTarget {
  const { server, model } = settings.actions[action];
  const external = server === "external";
  return {
    apiUrl: (external ? settings.externalUrl : settings.localUrl).trim(),
    model: model.trim(),
    apiKey: external ? settings.externalKey.trim() || null : null
  };
}

/** Короткая подпись для панели: какая модель и где выполнит действие. */
export function actionLabel(settings: LlmSettings, action: LlmAction) {
  const { server, model } = settings.actions[action];
  return `${model.trim() || "модель не выбрана"}, ${server === "external" ? "внешний сервер" : "локальный сервер"}`;
}
