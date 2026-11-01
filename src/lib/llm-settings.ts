export type LlmProvider = "local" | "openai" | "claude" | "gemini" | "compatible";
export type LlmAction = "descriptions" | "names" | "terms" | "termSearch" | "tokens";

const LOCAL_URL = "http://127.0.0.1:1234/v1";

export const LLM_PROVIDERS: { id: LlmProvider; label: string; url: string; urlHint: string; keyHint: string }[] = [
  { id: "local", label: "Локальный сервер", url: LOCAL_URL, urlHint: "Адрес (LM Studio, Ollama)", keyHint: "" },
  { id: "openai", label: "ChatGPT", url: "https://api.openai.com/v1", urlHint: "Адрес", keyHint: "Ключ создаётся на platform.openai.com/api-keys" },
  { id: "claude", label: "Claude", url: "https://api.anthropic.com", urlHint: "Адрес", keyHint: "Ключ создаётся на platform.claude.com/settings/keys" },
  { id: "gemini", label: "Gemini", url: "https://generativelanguage.googleapis.com/v1beta/openai", urlHint: "Адрес", keyHint: "Ключ создаётся на aistudio.google.com/apikey" },
  { id: "compatible", label: "Другой сервер", url: "", urlHint: "Адрес (OpenAI-совместимый API или сервер по IP)", keyHint: "" }
];

export const LLM_ACTIONS: { id: LlmAction; label: string; sends: string; gameText: boolean }[] = [
  { id: "descriptions", label: "Описания персонажей", sends: "сцены с репликами", gameText: true },
  { id: "names", label: "Имена", sends: "только список имён", gameText: false },
  { id: "terms", label: "Перевод терминов", sends: "только термины", gameText: false },
  { id: "termSearch", label: "Поиск терминов", sends: "названия и примеры реплик", gameText: true },
  { id: "tokens", label: "Токены", sends: "реплики с токенами", gameText: true }
];

export interface LlmConnection {
  id: string;
  name: string;
  provider: LlmProvider;
  url: string;
  key: string;
  /** Модель, которую получают действия этого подключения, пока не выбрали свою. */
  model: string;
}

/** Выбор действия. Пустое поле значит значение по умолчанию. */
export interface LlmActionSettings {
  connection: string;
  model: string;
}

export interface LlmSettings {
  connections: LlmConnection[];
  defaultConnection: string;
  actions: Record<LlmAction, LlmActionSettings>;
}

export interface LlmTarget {
  apiUrl: string;
  model: string;
  apiKey: string | null;
  provider: ApiProvider;
}

export type ApiProvider = Exclude<LlmProvider, "local">;

/** То, что сохранено сейчас, вместе с полями прежней версии с одним локальным и одним внешним сервером. */
interface StoredSettings {
  connections?: Partial<LlmConnection>[];
  defaultConnection?: string;
  localUrl?: string;
  externalUrl?: string;
  externalKey?: string;
  actions?: Partial<Record<LlmAction, { connection?: string; server?: string; model?: string }>>;
}

const SETTINGS_KEY = "translator.llm_settings";
const DEFAULT_MODEL = "mistral-small";

const providerOf = (id: LlmProvider) => LLM_PROVIDERS.find((provider) => provider.id === id)!;

function storedConnection(connection: Partial<LlmConnection>, index: number): LlmConnection {
  const provider = LLM_PROVIDERS.some(({ id }) => id === connection.provider) ? connection.provider! : "compatible";
  return {
    id: connection.id || `${provider}-${index + 1}`,
    name: connection.name || providerOf(provider).label,
    provider,
    url: connection.url ?? "",
    key: connection.key ?? "",
    model: connection.model ?? ""
  };
}

/** Настройки из сохранённого JSON. Без него берутся значения прежних версий из `legacy`. */
export function parseLlmSettings(saved: string | null, legacy: (key: string) => string | null): LlmSettings {
  let stored: StoredSettings = {};
  try {
    stored = saved ? JSON.parse(saved) : {};
  } catch {
    // Повреждённые настройки заменяются обычными
  }
  const localModel = legacy("translator.llm_model")?.trim() || DEFAULT_MODEL;
  const oldNamesUrl = legacy("translator.llm_names_api_url")?.trim() ?? "";
  const oldNamesModel = legacy("translator.llm_names_model")?.trim() ?? "";

  let connections = Array.isArray(stored.connections) ? stored.connections.map(storedConnection) : [];
  if (connections.length === 0) {
    connections = [{
      id: "local",
      name: providerOf("local").label,
      provider: "local",
      url: stored.localUrl ?? legacy("translator.llm_api_url") ?? LOCAL_URL,
      key: "",
      model: ""
    }];
    const externalUrl = stored.externalUrl ?? oldNamesUrl;
    const externalKey = stored.externalKey ?? legacy("translator.llm_names_api_key") ?? "";
    if (externalUrl || externalKey) {
      connections.push({ id: "external", name: "Внешний сервер", provider: "compatible", url: externalUrl, key: externalKey, model: "" });
    }
  }

  const known = (id: string | undefined) => (connections.some((connection) => connection.id === id) ? id! : connections[0].id);
  const fallback = (id: LlmAction): LlmActionSettings =>
    id === "names" && oldNamesUrl ? { connection: "external", model: oldNamesModel } : { connection: "", model: "" };
  const actions = Object.fromEntries(
    LLM_ACTIONS.map(({ id }) => {
      const action = stored.actions?.[id];
      if (!action) return [id, fallback(id)];
      const connection = action.connection ?? action.server;
      return [id, { connection: connection ? known(connection) : "", model: action.model ?? "" }];
    })
  ) as Record<LlmAction, LlmActionSettings>;

  // Прежние настройки хранили модель в действиях: подключение получает первую из них
  connections.forEach((connection, index) => {
    if (stored.connections?.[index]?.model !== undefined) return;
    const used = Object.values(actions).find((action) => action.connection === connection.id && action.model);
    connection.model = used?.model ?? (connection.provider === "local" ? localModel : "");
  });
  const defaultConnection = known(stored.defaultConnection ?? actions.descriptions.connection);
  return { connections, defaultConnection, actions };
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

/** Новое подключение с адресом сервиса. Второе подключение к тому же сервису получает номер. */
export function newConnection(provider: LlmProvider, connections: LlmConnection[]): LlmConnection {
  const { label, url } = providerOf(provider);
  let number = 1;
  while (connections.some((connection) => connection.id === `${provider}-${number}`)) number += 1;
  return { id: `${provider}-${number}`, name: number === 1 ? label : `${label} ${number}`, provider, url, key: "", model: "" };
}

/** Убирает подключение. Его действия возвращаются к значениям по умолчанию. */
export function withoutConnection(settings: LlmSettings, id: string): LlmSettings {
  const connections = settings.connections.filter((connection) => connection.id !== id);
  const actions = Object.fromEntries(
    Object.entries(settings.actions).map(([action, choice]) => [
      action,
      choice.connection === id ? { connection: "", model: "" } : choice
    ])
  ) as Record<LlmAction, LlmActionSettings>;
  return { connections, defaultConnection: settings.defaultConnection === id ? connections[0].id : settings.defaultConnection, actions };
}

// Имена без признаков типа: сервисы не сообщают в списке, что модель умеет. «instruct» целиком не годится:
// так называются и обычные локальные модели
const NOT_TEXT_MODEL = /embed|\btts\b|whisper|transcribe|dall-e|image|\bsora|\bveo|moderation|realtime|audio|speech|lyria|nano-banana|robotics|turbo-instruct|babbage|davinci/i;

/** Модели, которые пишут текст, по имени: числа сравниваются как числа, поэтому 4.1 идёт раньше 10. */
export function textModels(models: string[]) {
  return models
    .filter((model) => !NOT_TEXT_MODEL.test(model))
    .sort((a, b) => a.localeCompare(b, "en", { numeric: true, sensitivity: "base" }));
}

/** Можно ли спросить список моделей: облачным сервисам без ключа отвечать нечем. */
export function readyToCheck({ provider, url, key }: LlmConnection) {
  return url.trim() !== "" && (key.trim() !== "" || providerOf(provider).keyHint === "");
}

/** Локальному серверу для запроса важно только то, что он совместим с OpenAI. */
export const apiProvider = (provider: LlmProvider): ApiProvider => (provider === "local" ? "compatible" : provider);

export function defaultConnectionOf(settings: LlmSettings) {
  return settings.connections.find((connection) => connection.id === settings.defaultConnection) ?? settings.connections[0];
}

/** Подключение действия: его собственный выбор или подключение по умолчанию. */
export function connectionFor(settings: LlmSettings, action: LlmAction) {
  return settings.connections.find((connection) => connection.id === settings.actions[action].connection) ?? defaultConnectionOf(settings);
}

/** Предупреждение о тексте игры, который уйдёт на чужой сервер. Пустая строка значит, что он никуда не уходит. */
export function outsideNote(settings: LlmSettings, action: LlmAction) {
  const connection = connectionFor(settings, action);
  const info = LLM_ACTIONS.find(({ id }) => id === action);
  if (connection.provider === "local" || !info?.gameText) return "";
  return `Наружу, в ${connection.name}, уйдут: ${info.sends}`;
}

/** Куда отправить запрос действия. */
export function targetFor(settings: LlmSettings, action: LlmAction): LlmTarget {
  const connection = connectionFor(settings, action);
  return {
    apiUrl: connection.url.trim(),
    model: settings.actions[action].model.trim() || connection.model.trim(),
    apiKey: connection.key.trim() || null,
    provider: apiProvider(connection.provider)
  };
}
