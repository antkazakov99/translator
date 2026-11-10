-- «Новые термины» по тексту игры заменил разбор сцены моделью, а перевод реплик больше не пишет
-- предположения терминов. Записанные предположения становятся обычными терминами.
DROP TABLE term_candidates;
DELETE FROM derived WHERE name = 'term_candidates';
ALTER TABLE terms DROP COLUMN guess;
