-- Глубина чтения проекта для описаний моделью. Одна строка; без неё действует значение из кода.
CREATE TABLE reading_depth (
  id        INTEGER PRIMARY KEY CHECK (id = 1),
  min_parts INTEGER NOT NULL CHECK (min_parts >= 1),
  max_parts INTEGER NOT NULL CHECK (max_parts >= min_parts),
  percent   INTEGER NOT NULL CHECK (percent BETWEEN 1 AND 100)
) STRICT;
