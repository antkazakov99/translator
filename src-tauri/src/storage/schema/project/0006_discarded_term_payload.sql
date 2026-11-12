-- Корзина терминов хранит термин целиком: возврат из корзины восстанавливает перевод, категорию, описание и формы.
-- Пустая категория: в корзине только название (мусор от разбора сцены, название, переданное персонажу,
-- строки до этого шага). Возврат такой строки создаёт термин категории general без перевода.
ALTER TABLE discarded_terms ADD COLUMN category TEXT
  CHECK (category IN ('general', 'location', 'organization', 'title', 'item', 'event'));
ALTER TABLE discarded_terms ADD COLUMN translation TEXT;
ALTER TABLE discarded_terms ADD COLUMN description TEXT;
ALTER TABLE discarded_terms ADD COLUMN forms TEXT;  -- JSON: [{"text": "PPF", "translation": "НП"}]
