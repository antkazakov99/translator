-- cache.db: большие индексы проекта, которые можно удалить. Описание в Docmost, страница cache.db.
-- Миграций нет: версия формата лежит в PRAGMA user_version, при несовпадении база пересоздаётся.

-- Из какого считывания и по какой ревизии поиска собран кэш: 'scan_id', 'search_rev'.
CREATE TABLE meta (
  key   TEXT NOT NULL PRIMARY KEY,
  value TEXT NOT NULL
) STRICT;

-- Разбор файла движком по хэшу содержимого.
CREATE TABLE parsed_files (
  content_hash   TEXT NOT NULL PRIMARY KEY,  -- как files.hash в project.db
  deps_hash      TEXT NOT NULL,          -- хэш файлов, от которых зависит разбор; '' — ни от каких
  parser_version INTEGER NOT NULL,       -- версия разбора движка
  data           BLOB NOT NULL           -- разбор от движка
) STRICT;

-- Поиск по оригиналу и действующему переводу текущего снимка. rowid = segments.id в project.db.
CREATE VIRTUAL TABLE segments_fts USING fts5(
  source_text,
  translation,
  tokenize = 'unicode61 remove_diacritics 2'
);
