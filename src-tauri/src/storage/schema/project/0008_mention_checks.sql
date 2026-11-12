-- Ответы модели из разбора сцены: упомянута ли запись в реплике или там обычное слово. Реплика узнаётся
-- по отпечатку оригинала (segments.source_hash). Ответ уходит вместе с записью.
CREATE TABLE character_mention_checks (
  character_id INTEGER NOT NULL REFERENCES characters (id) ON DELETE CASCADE,
  text_hash    TEXT NOT NULL,
  used         INTEGER NOT NULL CHECK (used IN (0, 1)),
  checked_at   INTEGER NOT NULL,
  PRIMARY KEY (character_id, text_hash)
) STRICT;

CREATE TABLE term_mention_checks (
  term_id      INTEGER NOT NULL REFERENCES terms (id) ON DELETE CASCADE,
  text_hash    TEXT NOT NULL,
  used         INTEGER NOT NULL CHECK (used IN (0, 1)),
  checked_at   INTEGER NOT NULL,
  PRIMARY KEY (term_id, text_hash)
) STRICT;
