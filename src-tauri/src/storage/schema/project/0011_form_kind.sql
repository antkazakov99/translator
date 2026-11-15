-- Вид формы персонажа: другое написание имени, другая фамилия или остальное (прозвище, титул, обращение).
-- Главное имя и фамилия выбираются по частоте среди написаний своего вида.
ALTER TABLE character_forms ADD COLUMN kind TEXT NOT NULL DEFAULT 'other' CHECK (kind IN ('first', 'last', 'other'));
