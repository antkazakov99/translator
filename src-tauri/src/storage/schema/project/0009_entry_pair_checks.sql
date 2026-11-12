-- Вопрос «одно ли это» разбора сцены теперь задаётся и о терминах, и о паре из термина и персонажа.
-- Ответы хранятся так же, как у пар персонажей (character_pair_checks): отвеченная пара не спрашивается снова.
CREATE TABLE term_pair_checks (
  first_id   INTEGER NOT NULL REFERENCES terms (id) ON DELETE CASCADE,  -- меньший id
  second_id  INTEGER NOT NULL REFERENCES terms (id) ON DELETE CASCADE,  -- больший id
  same       INTEGER NOT NULL CHECK (same IN (0, 1)),
  checked_at INTEGER NOT NULL,
  PRIMARY KEY (first_id, second_id),
  CHECK (first_id < second_id)
) STRICT;

CREATE INDEX term_pair_checks_second ON term_pair_checks (second_id);

CREATE TABLE term_character_checks (
  term_id      INTEGER NOT NULL REFERENCES terms (id) ON DELETE CASCADE,
  character_id INTEGER NOT NULL REFERENCES characters (id) ON DELETE CASCADE,
  same         INTEGER NOT NULL CHECK (same IN (0, 1)),
  checked_at   INTEGER NOT NULL,
  PRIMARY KEY (term_id, character_id)
) STRICT;

CREATE INDEX term_character_checks_character ON term_character_checks (character_id);
