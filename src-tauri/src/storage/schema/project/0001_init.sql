-- project.db: данные одного проекта. Описание полей в Docmost, страница project.db.
-- Ключи контейнеров, реплик и говорящих задаёт движок, база хранит их как непрозрачные строки.

-- Проект и считывания ---------------------------------------------------------

CREATE TABLE project (
  id              TEXT NOT NULL PRIMARY KEY,  -- тот же UUID, что в app.db
  engine          TEXT NOT NULL,         -- код движка, как в app.db
  source_language TEXT,                  -- код BCP 47 оригинала, если известен
  target_language TEXT NOT NULL,         -- код BCP 47 перевода: 'ru'
  search_rev      INTEGER NOT NULL DEFAULT 0,  -- ревизия данных поиска
  created_at      INTEGER NOT NULL
) STRICT;

CREATE TABLE scans (
  id             INTEGER PRIMARY KEY,
  read_at        INTEGER NOT NULL,
  game_root      TEXT,                   -- откуда считали; NULL — пересборка из files/
  parser_version INTEGER NOT NULL,       -- версия разбора движка
  game_info      TEXT NOT NULL           -- JSON GameInfo: версия, данные движка для сборки мода
) STRICT;

-- Снимок игры -----------------------------------------------------------------

CREATE TABLE files (
  id             INTEGER PRIMARY KEY,
  path           TEXT NOT NULL,          -- от корня игры, через /
  kind           TEXT NOT NULL CHECK (kind IN ('script', 'translation', 'asset', 'font')),
  hash           TEXT NOT NULL,          -- SHA-256 содержимого, имя файла в files/
  size           INTEGER NOT NULL,
  last_seen_scan INTEGER NOT NULL REFERENCES scans (id),
  UNIQUE (path, hash)
) STRICT;

CREATE INDEX files_hash ON files (hash);

CREATE TABLE speakers (
  key            TEXT NOT NULL PRIMARY KEY,  -- от движка: 'code:lis', 'label:Radio Chatter'
  kind           TEXT NOT NULL,          -- значения задаёт движок: 'code', 'label'
  label          TEXT,                   -- подпись, которую показывает игра
  label_key      TEXT,                   -- NameKey подписи
  last_seen_scan INTEGER NOT NULL REFERENCES scans (id)
) STRICT;

CREATE TABLE containers (
  id             INTEGER PRIMARY KEY,
  key            TEXT NOT NULL UNIQUE,   -- ключ от движка, уникален во всей игре
  parent_id      INTEGER REFERENCES containers (id) ON DELETE CASCADE,
  kind           TEXT NOT NULL,          -- значения задаёт движок: 'scene', 'screen'
  file_id        INTEGER REFERENCES files (id) ON DELETE SET NULL,
  story_order    INTEGER,
  last_seen_scan INTEGER NOT NULL REFERENCES scans (id)
) STRICT;

CREATE INDEX containers_parent ON containers (parent_id, story_order);

CREATE TABLE segments (
  id             INTEGER PRIMARY KEY,
  key            TEXT NOT NULL,          -- ключ от движка, уникален среди текущих реплик
  missing        INTEGER NOT NULL DEFAULT 0,  -- 1 — пропала из игры
  container_id   INTEGER REFERENCES containers (id) ON DELETE SET NULL,
  file_id        INTEGER NOT NULL REFERENCES files (id),
  speaker_key    TEXT,                   -- ключ говорящего
  source_text    TEXT NOT NULL,
  source_hash    TEXT NOT NULL,
  game_text      TEXT,                   -- перевод, который уже есть в игре
  location       TEXT,                   -- строка в файле, только для показа
  story_order    INTEGER,
  engine_meta    TEXT,                   -- JSON, который читает только движок
  last_seen_scan INTEGER NOT NULL REFERENCES scans (id)
) STRICT;

CREATE UNIQUE INDEX segments_key ON segments (key) WHERE missing = 0;
CREATE INDEX segments_container ON segments (container_id, story_order);
CREATE INDEX segments_file      ON segments (file_id);
CREATE INDEX segments_speaker   ON segments (speaker_key);
CREATE INDEX segments_source    ON segments (source_hash);

-- Производные таблицы ---------------------------------------------------------

CREATE TABLE derived (
  name    TEXT NOT NULL PRIMARY KEY,     -- 'edges', 'term_candidates'
  version INTEGER NOT NULL,              -- версия кода, который собрал таблицу
  scan_id INTEGER NOT NULL REFERENCES scans (id)
) STRICT;

CREATE TABLE edges (
  id             INTEGER PRIMARY KEY,
  from_container TEXT NOT NULL,          -- ключ контейнера
  to_container   TEXT NOT NULL,
  kind           TEXT NOT NULL,          -- 'jump', 'call', 'screen', 'choice'
  condition      TEXT NOT NULL DEFAULT '',  -- '' — без условия
  UNIQUE (from_container, to_container, kind, condition)
) STRICT;

CREATE INDEX edges_to ON edges (to_container);

CREATE TABLE term_candidates (
  text_key    TEXT NOT NULL PRIMARY KEY, -- NameKey названия
  text        TEXT NOT NULL,             -- написание для показа
  occurrences INTEGER NOT NULL,          -- в скольких разных репликах встретилось
  example     TEXT NOT NULL              -- реплика для показа
) STRICT;

-- Запуски модели и перевод ----------------------------------------------------

CREATE TABLE llm_runs (
  id          INTEGER PRIMARY KEY,
  action_id   TEXT NOT NULL,             -- действие из справочника app.db
  provider    TEXT NOT NULL,             -- имя провайдера на момент запуска
  model       TEXT NOT NULL,
  status      TEXT NOT NULL CHECK (status IN ('running', 'done', 'cancelled', 'failed')),
  error       TEXT,
  started_at  INTEGER NOT NULL,
  finished_at INTEGER
) STRICT;

CREATE UNIQUE INDEX llm_runs_running ON llm_runs (status) WHERE status = 'running';

CREATE TABLE translations (
  segment_id  INTEGER PRIMARY KEY REFERENCES segments (id) ON DELETE CASCADE,
  text        TEXT NOT NULL,             -- '' = очищено, запуск модели переведёт заново
  draft       INTEGER NOT NULL DEFAULT 0,-- перевод модели, который пользователь не проверил
  run_id      INTEGER REFERENCES llm_runs (id) ON DELETE SET NULL,
  source_hash TEXT NOT NULL,             -- для какого оригинала сделан перевод
  updated_at  INTEGER NOT NULL
) STRICT;

CREATE INDEX translations_draft ON translations (draft) WHERE draft = 1;

CREATE TABLE revisions (
  id                 INTEGER PRIMARY KEY,
  segment_id         INTEGER NOT NULL REFERENCES segments (id) ON DELETE CASCADE,
  kind               TEXT NOT NULL CHECK (kind IN ('user', 'llm', 'rollback', 'transfer')),
  had_row            INTEGER NOT NULL,   -- была ли строка translations до правки
  text_before        TEXT,               -- '' — была очищена
  draft_before       INTEGER,
  source_hash_before TEXT,
  text_after         TEXT,               -- NULL — строка translations удалена
  run_id             INTEGER REFERENCES llm_runs (id) ON DELETE SET NULL,
  created_at         INTEGER NOT NULL
) STRICT;

CREATE INDEX revisions_segment ON revisions (segment_id, id);
CREATE INDEX revisions_run     ON revisions (run_id);

-- Глоссарий: говорящие и персонажи --------------------------------------------

CREATE TABLE speaker_entries (
  speaker_key TEXT NOT NULL PRIMARY KEY, -- ключ говорящего
  translation TEXT,                      -- перевод подписи
  gender      TEXT CHECK (gender IN ('male', 'female')),   -- NULL — неизвестен
  created_at  INTEGER NOT NULL
) STRICT;

CREATE TABLE characters (
  id                    INTEGER PRIMARY KEY,
  first_name            TEXT,
  first_name_key        TEXT,            -- NameKey имени
  last_name             TEXT,
  last_name_key         TEXT,            -- NameKey фамилии
  translated_first_name TEXT,
  translated_last_name  TEXT,
  gender                TEXT CHECK (gender IN ('male', 'female')),   -- NULL — неизвестен
  speech                TEXT,            -- манера речи
  description           TEXT
) STRICT;

CREATE TABLE speaker_characters (
  speaker_key  TEXT NOT NULL,            -- ключ говорящего
  character_id INTEGER NOT NULL REFERENCES characters (id) ON DELETE CASCADE,
  is_primary   INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (speaker_key, character_id)
) STRICT;

CREATE INDEX speaker_characters_character ON speaker_characters (character_id);
CREATE UNIQUE INDEX speaker_characters_primary ON speaker_characters (character_id) WHERE is_primary = 1;

CREATE TABLE discarded_speakers (
  key          TEXT NOT NULL PRIMARY KEY,
  discarded_at INTEGER NOT NULL
) STRICT;

CREATE TABLE character_forms (
  id           INTEGER PRIMARY KEY,
  character_id INTEGER NOT NULL REFERENCES characters (id) ON DELETE CASCADE,
  text         TEXT NOT NULL,
  text_key     TEXT NOT NULL,            -- NameKey формы
  translation  TEXT,
  UNIQUE (character_id, text_key)
) STRICT;

CREATE INDEX character_forms_text ON character_forms (text_key);

CREATE TABLE name_origins (
  character_id INTEGER NOT NULL REFERENCES characters (id) ON DELETE CASCADE,
  field        TEXT NOT NULL CHECK (field IN ('first', 'last', 'form')),
  source       TEXT NOT NULL,            -- разобранное написание
  context      TEXT NOT NULL,            -- полное имя персонажа во время разбора
  languages    TEXT NOT NULL,            -- JSON-массив кодов BCP 47: ["sr", "hr"]
  reason       TEXT,                     -- пояснение модели
  PRIMARY KEY (character_id, field, source)
) STRICT;

CREATE TABLE character_excluded_containers (
  character_id  INTEGER NOT NULL REFERENCES characters (id) ON DELETE CASCADE,
  container_key TEXT NOT NULL,
  PRIMARY KEY (character_id, container_key)
) STRICT;

-- Глоссарий: термины ----------------------------------------------------------

CREATE TABLE terms (
  id          INTEGER PRIMARY KEY,
  name        TEXT NOT NULL,
  name_key    TEXT NOT NULL UNIQUE,      -- NameKey названия
  translation TEXT,
  category    TEXT NOT NULL DEFAULT 'general'
    CHECK (category IN ('general', 'location', 'organization', 'title', 'item', 'event')),
  guess       INTEGER NOT NULL DEFAULT 0,  -- предположение модели, не подтверждено
  description TEXT
) STRICT;

CREATE TABLE term_forms (
  id          INTEGER PRIMARY KEY,
  term_id     INTEGER NOT NULL REFERENCES terms (id) ON DELETE CASCADE,
  text        TEXT NOT NULL,
  text_key    TEXT NOT NULL,             -- NameKey формы
  translation TEXT,
  UNIQUE (term_id, text_key)
) STRICT;

CREATE INDEX term_forms_text ON term_forms (text_key);

CREATE TABLE term_excluded_containers (
  term_id       INTEGER NOT NULL REFERENCES terms (id) ON DELETE CASCADE,
  container_key TEXT NOT NULL,
  PRIMARY KEY (term_id, container_key)
) STRICT;

CREATE TABLE discarded_terms (
  text_key     TEXT NOT NULL PRIMARY KEY,  -- NameKey названия
  text         TEXT NOT NULL,            -- написание для показа
  reason       TEXT NOT NULL CHECK (reason IN ('junk', 'given_to_character')),
  discarded_at INTEGER NOT NULL
) STRICT;

-- Имена глоссария, которые раздел «Новые термины» вычитает из кандидатов.
CREATE VIEW glossary_name_keys AS
  SELECT first_name_key AS text_key FROM characters WHERE first_name_key IS NOT NULL
  UNION SELECT last_name_key FROM characters WHERE last_name_key IS NOT NULL
  UNION SELECT first_name_key || ' ' || last_name_key FROM characters
    WHERE first_name_key IS NOT NULL AND last_name_key IS NOT NULL
  UNION SELECT text_key FROM character_forms
  UNION SELECT s.label_key FROM speakers s
    WHERE s.label_key IS NOT NULL
      AND (s.key IN (SELECT speaker_key FROM speaker_characters)
           OR s.key IN (SELECT speaker_key FROM speaker_entries));

-- Сцены, токены, заметки ------------------------------------------------------

CREATE TABLE scene_descriptions (
  container_key TEXT NOT NULL PRIMARY KEY,  -- ключ контейнера
  title         TEXT,                    -- название сцены
  description   TEXT,                    -- описание сцены
  text_hash     TEXT,                    -- какой текст прочитала модель
  updated_at    INTEGER NOT NULL
) STRICT;

CREATE TABLE tokens (
  name  TEXT NOT NULL PRIMARY KEY,       -- без скобок: 'mc', 'friend.name'
  value TEXT,                            -- чем заменить токен при чтении сцен и переводе
  note  TEXT                             -- пояснение для модели, если значения нет
) STRICT;

CREATE TABLE notes (
  owner_kind TEXT NOT NULL CHECK (owner_kind IN ('character', 'term', 'container')),
  owner_ref  TEXT NOT NULL,              -- id персонажа или термина, ключ контейнера
  model      TEXT NOT NULL,
  part_hash  TEXT NOT NULL,              -- отпечаток текста части
  text       TEXT NOT NULL,
  excluded   INTEGER NOT NULL DEFAULT 0, -- пользователь исключил из описания
  created_at INTEGER NOT NULL,
  PRIMARY KEY (owner_kind, owner_ref, model, part_hash)
) STRICT;

-- Настройки проекта -----------------------------------------------------------

-- Переопределения настроек действий из app.db. Внешних ключей нет: справочник и профили в другой базе.
CREATE TABLE llm_action_settings (
  action_id   TEXT NOT NULL PRIMARY KEY,
  profile_id  TEXT,
  temperature REAL CHECK (temperature BETWEEN 0 AND 2),
  batch_size  INTEGER CHECK (batch_size > 0)
) STRICT;

CREATE TABLE name_rule_overrides (
  from_language TEXT NOT NULL PRIMARY KEY,  -- код BCP 47; язык перевода — project.target_language
  rules         TEXT NOT NULL
) STRICT;

CREATE TABLE font_replacements (
  target_path TEXT NOT NULL PRIMARY KEY, -- путь шрифта в игре, который заменяется
  file_name   TEXT NOT NULL,             -- имя загруженного файла, для показа
  hash        TEXT NOT NULL,             -- SHA-256, имя файла в files/
  added_at    INTEGER NOT NULL
) STRICT;
