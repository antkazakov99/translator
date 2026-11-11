-- Отметки для цепочки «Вся игра»: повторный запуск продолжает с того же места.

-- Части сцен, которые прочитал разбор (действие analysis): повторный разбор их пропускает.
CREATE TABLE analyzed_parts (
  container_key TEXT NOT NULL,
  part_hash     TEXT NOT NULL,           -- отпечаток текста части, как в notes
  analyzed_at   INTEGER NOT NULL,
  PRIMARY KEY (container_key, part_hash)
) STRICT;

-- Реплики, которые вычитала модель, и текст после вычитки. Другой текст значит «вычитать снова».
CREATE TABLE reviewed_lines (
  segment_id  INTEGER PRIMARY KEY REFERENCES segments (id) ON DELETE CASCADE,
  text        TEXT NOT NULL,
  reviewed_at INTEGER NOT NULL
) STRICT;
