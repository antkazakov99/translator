-- app.db: всё, что не относится к одному проекту. Описание полей в Docmost, страница app.db.

-- Проекты. Не зависят от папки игры: она только привязка.
CREATE TABLE projects (
  id         TEXT NOT NULL PRIMARY KEY,  -- UUID v7, он же имя папки данных проекта
  status     TEXT NOT NULL DEFAULT 'ready'
    CHECK (status IN ('creating', 'ready', 'deleting')),
  title      TEXT NOT NULL,              -- при создании имя корневой папки игры
  engine     TEXT NOT NULL,              -- код движка: 'renpy'
  game_path  TEXT,                       -- корневая папка игры; NULL — не привязана
  created_at INTEGER NOT NULL,           -- unix-время, секунды
  opened_at  INTEGER,                    -- порядок в списке
  archived   INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE INDEX projects_game_path ON projects (game_path);

-- Провайдеры: куда идёт запрос. Ключ API лежит в системном хранилище учётных данных.
CREATE TABLE llm_providers (
  id       TEXT NOT NULL PRIMARY KEY,    -- UUID v7, он же имя ключа в хранилище учётных данных
  position INTEGER NOT NULL,
  name     TEXT NOT NULL,
  kind     TEXT NOT NULL
    CHECK (kind IN ('local', 'openai', 'claude', 'gemini', 'compatible')),
  url      TEXT NOT NULL
) STRICT;

-- Профили: какая модель отвечает и с какими настройками. NULL значит значение из кода.
CREATE TABLE llm_profiles (
  id               TEXT NOT NULL PRIMARY KEY,  -- UUID v7
  provider_id      TEXT NOT NULL REFERENCES llm_providers (id) ON DELETE CASCADE,
  position         INTEGER NOT NULL,     -- порядок в общем списке профилей
  name             TEXT NOT NULL,
  model            TEXT NOT NULL,
  context_tokens   INTEGER CHECK (context_tokens > 0),
  reasoning_tokens INTEGER CHECK (reasoning_tokens >= 0),
  timeout_secs     INTEGER CHECK (timeout_secs > 0),
  params           TEXT                  -- JSON
) STRICT;

CREATE INDEX llm_profiles_provider ON llm_profiles (provider_id);

-- Справочник действий модели. Название, порядок и значения по умолчанию задаёт код (Action).
CREATE TABLE llm_actions (
  id TEXT NOT NULL PRIMARY KEY
) STRICT;

INSERT INTO llm_actions (id) VALUES
  ('descriptions'), ('names'), ('terms'), ('tokens'), ('speakers'), ('lines'), ('review');

-- Настройки действий. Нет строки или NULL в поле — значение по умолчанию.
CREATE TABLE llm_action_settings (
  action_id   TEXT NOT NULL PRIMARY KEY REFERENCES llm_actions (id) ON DELETE CASCADE,
  profile_id  TEXT REFERENCES llm_profiles (id) ON DELETE SET NULL,
  temperature REAL CHECK (temperature BETWEEN 0 AND 2),
  batch_size  INTEGER CHECK (batch_size > 0)
) STRICT;

-- Правила передачи имён по паре языков, общие на все игры.
CREATE TABLE name_rules (
  from_language TEXT NOT NULL,           -- код BCP 47 языка происхождения имени: 'sr'
  to_language   TEXT NOT NULL,           -- код BCP 47 языка перевода: 'ru'
  rules         TEXT NOT NULL,
  PRIMARY KEY (from_language, to_language)
) STRICT;
