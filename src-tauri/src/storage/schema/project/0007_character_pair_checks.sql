-- Пары персонажей, про которые разбор сцены спросил модель, один ли это человек: повторно не спрашиваются.
-- Слитая пара уходит вместе с удалённым персонажем.
CREATE TABLE character_pair_checks (
  first_id   INTEGER NOT NULL REFERENCES characters (id) ON DELETE CASCADE,  -- меньший id
  second_id  INTEGER NOT NULL REFERENCES characters (id) ON DELETE CASCADE,  -- больший id
  same       INTEGER NOT NULL CHECK (same IN (0, 1)),
  checked_at INTEGER NOT NULL,
  PRIMARY KEY (first_id, second_id),
  CHECK (first_id < second_id)
) STRICT;

CREATE INDEX character_pair_checks_second ON character_pair_checks (second_id);
