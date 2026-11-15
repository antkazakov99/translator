-- Цепочка автоперевода запоминает и персонажа или термин, который она описывала и не получила описания:
-- вид `description`. С другими частями (новые сцены записи, другая глубина чтения) запись спросится снова.
-- CHECK у готовой таблицы не меняется, поэтому таблица пересоздаётся.
CREATE TABLE glossary_asks_next (
  kind     TEXT NOT NULL CHECK (kind IN ('term', 'speaker', 'token', 'name', 'description')),
  key      TEXT NOT NULL,           -- term: ключ названия; speaker: ключ говорящего; token: имя;
                                    -- name: «номер персонажа:поле:ключ написания»;
                                    -- description: «character или term:номер записи:отпечаток выбранных частей»
  asked_at INTEGER NOT NULL,
  PRIMARY KEY (kind, key)
) STRICT;

INSERT INTO glossary_asks_next (kind, key, asked_at) SELECT kind, key, asked_at FROM glossary_asks;
DROP TABLE glossary_asks;
ALTER TABLE glossary_asks_next RENAME TO glossary_asks;
