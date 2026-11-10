-- Глубина чтения для описаний моделью: одна на приложение. Одна строка; без неё действует значение из кода.
-- max_parts NULL — без предела.
CREATE TABLE reading_depth (
  id        INTEGER PRIMARY KEY CHECK (id = 1),
  min_parts INTEGER NOT NULL CHECK (min_parts >= 1),
  max_parts INTEGER CHECK (max_parts IS NULL OR max_parts >= min_parts),
  percent   INTEGER NOT NULL CHECK (percent BETWEEN 1 AND 100)
) STRICT;
