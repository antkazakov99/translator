//! Таблицы глоссария в project.db.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, Row, params};

use super::terms::TermCandidate;
use super::{
    Character, CharacterId, DiscardReason, DiscardedTerm, Form, NameKey, NameOrigin, Note, NoteOwner, SceneDescription, Speaker,
    SpeakerEntry, SpeakerLink, Term, TermCategory, TermId, Token,
};
use crate::common::{Gender, Timestamp};
use crate::engines::{ContainerKey, SpeakerKey};
use crate::storage::DbError;

fn speaker_key(table: &'static str, key: String) -> Result<SpeakerKey, DbError> {
    SpeakerKey::new(key).map_err(|error| DbError::bad_value(table, "speaker_key", error))
}

fn gender(table: &'static str, text: Option<String>) -> Result<Option<Gender>, DbError> {
    text.map(|text| Gender::parse(&text)).transpose().map_err(|error| DbError::bad_value(table, "gender", error))
}

fn form(table: &'static str, text: &str, translation: Option<&str>) -> Result<Form, DbError> {
    Form::new(text, translation).map_err(|error| DbError::bad_value(table, "text", error))
}

/// Говорящий текущего снимка и число его реплик.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeakerUse {
    pub speaker: Speaker,
    pub lines: usize,
}

/// `speakers`, `speaker_entries`, `speaker_characters`, `discarded_speakers`.
pub struct SpeakerRepo<'c> {
    pub conn: &'c Connection,
}

impl SpeakerRepo<'_> {
    /// Говорящие текущего снимка с числом реплик, в порядке подписи.
    pub fn current(&self) -> Result<Vec<SpeakerUse>, DbError> {
        let mut statement = self.conn.prepare(
            "SELECT s.key, s.kind, s.label,
                    (SELECT count(*) FROM segments g WHERE g.speaker_key = s.key AND g.missing = 0)
             FROM speakers s
             WHERE s.last_seen_scan = (SELECT max(id) FROM scans)
             ORDER BY coalesce(s.label_key, s.key)",
        )?;
        let rows = statement
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get(1)?, row.get(2)?, row.get::<_, i64>(3)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(key, kind, label, lines)| {
                Ok(SpeakerUse { speaker: Speaker { key: speaker_key("speakers", key)?, kind, label }, lines: lines as usize })
            })
            .collect()
    }

    /// Говорящие текущего снимка, которых пользователь ещё не разобрал.
    pub fn new_speakers(&self) -> Result<Vec<Speaker>, DbError> {
        let mut statement = self.conn.prepare(
            "SELECT s.key, s.kind, s.label
             FROM speakers s
             WHERE s.last_seen_scan = (SELECT max(id) FROM scans)
               AND s.key NOT IN (SELECT speaker_key FROM speaker_characters)
               AND s.key NOT IN (SELECT speaker_key FROM speaker_entries)
               AND s.key NOT IN (SELECT key FROM discarded_speakers)
             ORDER BY coalesce(s.label_key, s.key)",
        )?;
        let rows = statement
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter().map(|(key, kind, label)| Ok(Speaker { key: speaker_key("speakers", key)?, kind, label })).collect()
    }

    /// Говорящий снимка по ключу: подпись нужна, чтобы назвать персонажа.
    pub fn get(&self, key: &SpeakerKey) -> Result<Speaker, DbError> {
        let row = self
            .conn
            .query_row("SELECT kind, label FROM speakers WHERE key = ?1", [key.as_str()], |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()?;
        let (kind, label) = row.ok_or(DbError::NotFound)?;
        Ok(Speaker { key: key.clone(), kind, label })
    }

    pub fn entries(&self) -> Result<Vec<SpeakerEntry>, DbError> {
        let mut statement = self.conn.prepare("SELECT speaker_key, translation, gender, created_at FROM speaker_entries")?;
        let rows = statement
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get(1)?, row.get(2)?, row.get::<_, i64>(3)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(key, translation, gender_text, created)| {
                Ok(SpeakerEntry::restore(
                    speaker_key("speaker_entries", key)?,
                    translation,
                    gender("speaker_entries", gender_text)?,
                    Timestamp::from_secs(created),
                ))
            })
            .collect()
    }

    pub fn save_entry(&self, entry: &SpeakerEntry) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT INTO speaker_entries (speaker_key, translation, gender, created_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (speaker_key) DO UPDATE SET translation = excluded.translation, gender = excluded.gender",
            params![entry.key().as_str(), entry.translation(), entry.gender().map(Gender::as_str), entry.created_at().secs()],
        )?;
        Ok(())
    }

    pub fn delete_entry(&self, key: &SpeakerKey) -> Result<(), DbError> {
        self.conn.execute("DELETE FROM speaker_entries WHERE speaker_key = ?1", [key.as_str()])?;
        Ok(())
    }

    /// Все связи говорящих с персонажами.
    pub fn links(&self) -> Result<Vec<SpeakerLink>, DbError> {
        self.links_where("", [])
    }

    pub fn links_of(&self, character: CharacterId) -> Result<Vec<SpeakerLink>, DbError> {
        self.links_where("WHERE character_id = ?1", [character.get()])
    }

    fn links_where<P: rusqlite::Params>(&self, filter: &str, params: P) -> Result<Vec<SpeakerLink>, DbError> {
        let sql = format!("SELECT speaker_key, character_id, is_primary FROM speaker_characters {filter} ORDER BY speaker_key");
        let rows = self
            .conn
            .prepare(&sql)?
            .query_map(params, |row| Ok((row.get::<_, String>(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(key, character, is_primary)| {
                Ok(SpeakerLink { speaker: speaker_key("speaker_characters", key)?, character: CharacterId::from_row(character), is_primary })
            })
            .collect()
    }

    /// Основной код у персонажа один: связь с `is_primary` снимает его с прежнего.
    pub fn link(&self, link: &SpeakerLink) -> Result<(), DbError> {
        if link.is_primary {
            self.conn.execute(
                "UPDATE speaker_characters SET is_primary = 0 WHERE character_id = ?1 AND speaker_key <> ?2",
                params![link.character.get(), link.speaker.as_str()],
            )?;
        }
        self.conn.execute(
            "INSERT INTO speaker_characters (speaker_key, character_id, is_primary) VALUES (?1, ?2, ?3)
             ON CONFLICT (speaker_key, character_id) DO UPDATE SET is_primary = excluded.is_primary",
            params![link.speaker.as_str(), link.character.get(), link.is_primary],
        )?;
        Ok(())
    }

    pub fn unlink(&self, speaker: &SpeakerKey, character: CharacterId) -> Result<(), DbError> {
        self.conn.execute(
            "DELETE FROM speaker_characters WHERE speaker_key = ?1 AND character_id = ?2",
            params![speaker.as_str(), character.get()],
        )?;
        Ok(())
    }

    /// Говорящие из `keys` без связей и записи: после удаления персонажа уходят в корзину.
    pub fn orphans(&self, keys: &[SpeakerKey]) -> Result<Vec<SpeakerKey>, DbError> {
        let mut orphans = Vec::new();
        for key in keys {
            let used: bool = self.conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM speaker_characters WHERE speaker_key = ?1)
                     OR EXISTS (SELECT 1 FROM speaker_entries WHERE speaker_key = ?1)",
                [key.as_str()],
                |row| row.get(0),
            )?;
            if !used {
                orphans.push(key.clone());
            }
        }
        Ok(orphans)
    }

    /// Корзина говорящих с подписями из снимка.
    pub fn discarded(&self) -> Result<Vec<Speaker>, DbError> {
        let mut statement = self.conn.prepare(
            "SELECT d.key, coalesce(s.kind, ''), s.label FROM discarded_speakers d
             LEFT JOIN speakers s ON s.key = d.key ORDER BY d.discarded_at DESC",
        )?;
        let rows = statement
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter().map(|(key, kind, label)| Ok(Speaker { key: speaker_key("discarded_speakers", key)?, kind, label })).collect()
    }

    pub fn discard(&self, key: &SpeakerKey, now: Timestamp) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT INTO discarded_speakers (key, discarded_at) VALUES (?1, ?2) ON CONFLICT (key) DO NOTHING",
            params![key.as_str(), now.secs()],
        )?;
        Ok(())
    }

    pub fn restore_discarded(&self, key: &SpeakerKey) -> Result<(), DbError> {
        self.conn.execute("DELETE FROM discarded_speakers WHERE key = ?1", [key.as_str()])?;
        Ok(())
    }
}

/// `characters`, `character_forms`, `name_origins`, `character_excluded_containers`.
pub struct CharacterRepo<'c> {
    pub conn: &'c Connection,
}

const CHARACTER_SELECT: &str = "SELECT id, first_name, last_name, translated_first_name, translated_last_name, gender, speech, description
                                FROM characters";

impl CharacterRepo<'_> {
    pub fn get(&self, id: CharacterId) -> Result<Character, DbError> {
        let sql = format!("{CHARACTER_SELECT} WHERE id = ?1");
        let row = self.conn.query_row(&sql, [id.get()], CharacterRow::read).optional()?;
        let mut forms = self.forms(Some(id))?;
        row.ok_or(DbError::NotFound)?.into_character(&mut forms)
    }

    /// По имени и фамилии.
    pub fn list(&self) -> Result<Vec<Character>, DbError> {
        let sql = format!("{CHARACTER_SELECT} ORDER BY coalesce(first_name_key, last_name_key), last_name_key, id");
        let rows = self.conn.prepare(&sql)?.query_map([], CharacterRow::read)?.collect::<Result<Vec<_>, _>>()?;
        let mut forms = self.forms(None)?;
        rows.into_iter().map(|row| row.into_character(&mut forms)).collect()
    }

    /// Формы по персонажам: одного или всех.
    fn forms(&self, only: Option<CharacterId>) -> Result<HashMap<i64, Vec<Form>>, DbError> {
        let mut statement = self.conn.prepare(
            "SELECT character_id, text, translation FROM character_forms WHERE ?1 IS NULL OR character_id = ?1 ORDER BY id",
        )?;
        let rows = statement
            .query_map([only.map(CharacterId::get)], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        let mut forms: HashMap<i64, Vec<Form>> = HashMap::new();
        for (id, text, translation) in rows {
            forms.entry(id).or_default().push(form("character_forms", &text, translation.as_deref())?);
        }
        Ok(forms)
    }

    /// Вставляет нового или заменяет поля и формы целиком. Возвращает id.
    pub fn save(&self, character: &Character) -> Result<CharacterId, DbError> {
        let key = |key: Option<NameKey>| key.map(|key| key.as_str().to_string());
        let id = match character.id() {
            Some(id) => id,
            None => {
                self.conn.execute("INSERT INTO characters DEFAULT VALUES", [])?;
                CharacterId::from_row(self.conn.last_insert_rowid())
            }
        };
        let changed = self.conn.execute(
            "UPDATE characters SET first_name = ?1, first_name_key = ?2, last_name = ?3, last_name_key = ?4,
               translated_first_name = ?5, translated_last_name = ?6, gender = ?7, speech = ?8, description = ?9
             WHERE id = ?10",
            params![
                character.first_name(),
                key(character.first_name_key()),
                character.last_name(),
                key(character.last_name_key()),
                character.translated_first_name(),
                character.translated_last_name(),
                character.gender().map(Gender::as_str),
                character.speech(),
                character.description(),
                id.get()
            ],
        )?;
        if changed == 0 {
            return Err(DbError::NotFound);
        }
        self.conn.execute("DELETE FROM character_forms WHERE character_id = ?1", [id.get()])?;
        for form in character.forms() {
            self.conn.execute(
                "INSERT INTO character_forms (character_id, text, text_key, translation) VALUES (?1, ?2, ?3, ?4)",
                params![id.get(), form.text(), form.key().as_str(), form.translation()],
            )?;
        }
        Ok(id)
    }

    /// Связи и формы удаляются каскадом, заметки удаляет сервис в той же транзакции.
    pub fn delete(&self, id: CharacterId) -> Result<(), DbError> {
        self.conn.execute("DELETE FROM characters WHERE id = ?1", [id.get()])?;
        Ok(())
    }

    pub fn origins(&self, id: CharacterId) -> Result<Vec<NameOrigin>, DbError> {
        todo!()
    }

    pub fn save_origins(&self, id: CharacterId, origins: &[NameOrigin]) -> Result<(), DbError> {
        todo!()
    }

    pub fn excluded_containers(&self, id: CharacterId) -> Result<Vec<ContainerKey>, DbError> {
        todo!()
    }

    pub fn set_excluded(&self, id: CharacterId, container: &ContainerKey, excluded: bool) -> Result<(), DbError> {
        todo!()
    }
}

/// Строка `characters` до сборки модели с формами.
struct CharacterRow {
    id: i64,
    first_name: Option<String>,
    last_name: Option<String>,
    translated_first_name: Option<String>,
    translated_last_name: Option<String>,
    gender: Option<String>,
    speech: Option<String>,
    description: Option<String>,
}

impl CharacterRow {
    fn read(row: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            first_name: row.get(1)?,
            last_name: row.get(2)?,
            translated_first_name: row.get(3)?,
            translated_last_name: row.get(4)?,
            gender: row.get(5)?,
            speech: row.get(6)?,
            description: row.get(7)?,
        })
    }

    fn into_character(self, forms: &mut HashMap<i64, Vec<Form>>) -> Result<Character, DbError> {
        Ok(Character::restore(
            CharacterId::from_row(self.id),
            self.first_name,
            self.last_name,
            self.translated_first_name,
            self.translated_last_name,
            gender("characters", self.gender)?,
            self.speech,
            self.description,
            forms.remove(&self.id).unwrap_or_default(),
        ))
    }
}

/// `terms`, `term_forms`, `term_excluded_containers`, `discarded_terms`, `term_candidates`.
pub struct TermRepo<'c> {
    pub conn: &'c Connection,
}

const TERM_SELECT: &str = "SELECT id, name, translation, category, guess, description FROM terms";

type TermRow = (i64, String, Option<String>, String, bool, Option<String>);

impl TermRepo<'_> {
    pub fn get(&self, id: TermId) -> Result<Term, DbError> {
        let sql = format!("{TERM_SELECT} WHERE id = ?1");
        let row = self.conn.query_row(&sql, [id.get()], Self::read).optional()?.ok_or(DbError::NotFound)?;
        let mut forms = self.forms(Some(id))?;
        Self::term(row, &mut forms)
    }

    /// По названию.
    pub fn list(&self) -> Result<Vec<Term>, DbError> {
        let sql = format!("{TERM_SELECT} ORDER BY name_key");
        let rows = self.conn.prepare(&sql)?.query_map([], Self::read)?.collect::<Result<Vec<_>, _>>()?;
        let mut forms = self.forms(None)?;
        rows.into_iter().map(|row| Self::term(row, &mut forms)).collect()
    }

    fn read(row: &Row) -> rusqlite::Result<TermRow> {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?))
    }

    fn term((id, name, translation, category, guess, description): TermRow, forms: &mut HashMap<i64, Vec<Form>>) -> Result<Term, DbError> {
        let category = TermCategory::parse(&category).map_err(|error| DbError::bad_value("terms", "category", error))?;
        let forms = forms.remove(&id).unwrap_or_default();
        Term::restore(TermId::from_row(id), &name, translation, category, guess, description, forms)
            .map_err(|error| DbError::bad_value("terms", "name", error))
    }

    fn forms(&self, only: Option<TermId>) -> Result<HashMap<i64, Vec<Form>>, DbError> {
        let mut statement =
            self.conn.prepare("SELECT term_id, text, translation FROM term_forms WHERE ?1 IS NULL OR term_id = ?1 ORDER BY id")?;
        let rows = statement
            .query_map([only.map(TermId::get)], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        let mut forms: HashMap<i64, Vec<Form>> = HashMap::new();
        for (id, text, translation) in rows {
            forms.entry(id).or_default().push(form("term_forms", &text, translation.as_deref())?);
        }
        Ok(forms)
    }

    /// Вставляет новый или заменяет поля и формы целиком. Название занято другим термином: `DbError` от `UNIQUE`.
    pub fn save(&self, term: &Term) -> Result<TermId, DbError> {
        let id = match term.id() {
            Some(id) => {
                let changed = self.conn.execute(
                    "UPDATE terms SET name = ?1, name_key = ?2, translation = ?3, category = ?4, guess = ?5, description = ?6
                     WHERE id = ?7",
                    params![
                        term.name(),
                        term.name_key().as_str(),
                        term.translation(),
                        term.category().as_str(),
                        term.is_guess(),
                        term.description(),
                        id.get()
                    ],
                )?;
                if changed == 0 {
                    return Err(DbError::NotFound);
                }
                id
            }
            None => {
                self.conn.execute(
                    "INSERT INTO terms (name, name_key, translation, category, guess, description) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        term.name(),
                        term.name_key().as_str(),
                        term.translation(),
                        term.category().as_str(),
                        term.is_guess(),
                        term.description()
                    ],
                )?;
                TermId::from_row(self.conn.last_insert_rowid())
            }
        };
        self.conn.execute("DELETE FROM term_forms WHERE term_id = ?1", [id.get()])?;
        for form in term.forms() {
            self.conn.execute(
                "INSERT INTO term_forms (term_id, text, text_key, translation) VALUES (?1, ?2, ?3, ?4)",
                params![id.get(), form.text(), form.key().as_str(), form.translation()],
            )?;
        }
        Ok(id)
    }

    pub fn delete(&self, id: TermId) -> Result<(), DbError> {
        self.conn.execute("DELETE FROM terms WHERE id = ?1", [id.get()])?;
        Ok(())
    }

    /// «Новые термины»: кандидаты без терминов, имён персонажей, подписей разобранных говорящих и корзины.
    pub fn new_candidates(&self) -> Result<Vec<TermCandidate>, DbError> {
        todo!()
    }

    /// Пересобирает `term_candidates` и строку `derived` одной транзакцией.
    pub fn replace_candidates(&self, candidates: &[TermCandidate], version: u32) -> Result<(), DbError> {
        todo!()
    }

    /// Есть ли название среди терминов, их форм, имён глоссария и корзины: так проверяются предположения модели.
    pub fn is_known(&self, key: &NameKey) -> Result<bool, DbError> {
        Ok(self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM terms WHERE name_key = ?1)
                 OR EXISTS (SELECT 1 FROM term_forms WHERE text_key = ?1)
                 OR EXISTS (SELECT 1 FROM glossary_name_keys WHERE text_key = ?1)
                 OR EXISTS (SELECT 1 FROM discarded_terms WHERE text_key = ?1)",
            [key.as_str()],
            |row| row.get(0),
        )?)
    }

    pub fn discarded(&self) -> Result<Vec<DiscardedTerm>, DbError> {
        let mut statement =
            self.conn.prepare("SELECT text_key, text, reason, discarded_at FROM discarded_terms ORDER BY discarded_at DESC, text_key")?;
        let rows = statement
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(key, text, reason, at)| {
                Ok(DiscardedTerm {
                    text,
                    key: NameKey::from_stored(key),
                    reason: DiscardReason::parse(&reason).map_err(|error| DbError::bad_value("discarded_terms", "reason", error))?,
                    discarded_at: Timestamp::from_secs(at),
                })
            })
            .collect()
    }

    pub fn discard(&self, term: &DiscardedTerm) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT INTO discarded_terms (text_key, text, reason, discarded_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (text_key) DO UPDATE SET text = excluded.text, reason = excluded.reason, discarded_at = excluded.discarded_at",
            params![term.key.as_str(), term.text, term.reason.as_str(), term.discarded_at.secs()],
        )?;
        Ok(())
    }

    pub fn restore_discarded(&self, key: &NameKey) -> Result<(), DbError> {
        self.conn.execute("DELETE FROM discarded_terms WHERE text_key = ?1", [key.as_str()])?;
        Ok(())
    }

    pub fn excluded_containers(&self, id: TermId) -> Result<Vec<ContainerKey>, DbError> {
        todo!()
    }

    pub fn set_excluded(&self, id: TermId, container: &ContainerKey, excluded: bool) -> Result<(), DbError> {
        todo!()
    }
}

/// `tokens`.
pub struct TokenRepo<'c> {
    pub conn: &'c Connection,
}

impl TokenRepo<'_> {
    pub fn list(&self) -> Result<Vec<Token>, DbError> {
        let mut statement = self.conn.prepare("SELECT name, value, note FROM tokens ORDER BY name")?;
        let rows = statement
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(name, value, note)| Token::restore(&name, value, note).map_err(|error| DbError::bad_value("tokens", "name", error)))
            .collect()
    }

    /// Пустой токен удаляется.
    pub fn save(&self, token: &Token) -> Result<(), DbError> {
        if token.is_empty() {
            return self.delete(token.name());
        }
        self.conn.execute(
            "INSERT INTO tokens (name, value, note) VALUES (?1, ?2, ?3)
             ON CONFLICT (name) DO UPDATE SET value = excluded.value, note = excluded.note",
            params![token.name(), token.value(), token.note()],
        )?;
        Ok(())
    }

    pub fn delete(&self, name: &str) -> Result<(), DbError> {
        self.conn.execute("DELETE FROM tokens WHERE name = ?1", [name])?;
        Ok(())
    }
}

/// `scene_descriptions`.
pub struct SceneDescriptionRepo<'c> {
    pub conn: &'c Connection,
}

impl SceneDescriptionRepo<'_> {
    pub fn get(&self, key: &ContainerKey) -> Result<Option<SceneDescription>, DbError> {
        self.conn
            .query_row(
                "SELECT title, description, text_hash, updated_at FROM scene_descriptions WHERE container_key = ?1",
                [key.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get::<_, i64>(3)?)),
            )
            .optional()?
            .map(|(title, description, hash, at)| Ok(SceneDescription::restore(key.clone(), title, description, hash, Timestamp::from_secs(at))))
            .transpose()
    }

    pub fn list(&self) -> Result<Vec<SceneDescription>, DbError> {
        let mut statement =
            self.conn.prepare("SELECT container_key, title, description, text_hash, updated_at FROM scene_descriptions ORDER BY container_key")?;
        let rows = statement
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get::<_, i64>(4)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(key, title, description, hash, at)| {
                let key = ContainerKey::new(key).map_err(|error| DbError::bad_value("scene_descriptions", "container_key", error))?;
                Ok(SceneDescription::restore(key, title, description, hash, Timestamp::from_secs(at)))
            })
            .collect()
    }

    /// Пустое описание удаляет строку.
    pub fn save(&self, scene: &SceneDescription) -> Result<(), DbError> {
        if scene.is_empty() {
            self.conn.execute("DELETE FROM scene_descriptions WHERE container_key = ?1", [scene.key().as_str()])?;
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO scene_descriptions (container_key, title, description, text_hash, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (container_key) DO UPDATE SET title = excluded.title, description = excluded.description,
               text_hash = excluded.text_hash, updated_at = excluded.updated_at",
            params![scene.key().as_str(), scene.title(), scene.description(), scene.text_hash(), scene.updated_at().secs()],
        )?;
        Ok(())
    }
}

/// `notes`.
pub struct NoteRepo<'c> {
    pub conn: &'c Connection,
}

impl NoteRepo<'_> {
    pub fn of(&self, owner: &NoteOwner, model: &str) -> Result<Vec<Note>, DbError> {
        todo!()
    }

    pub fn save(&self, note: &Note) -> Result<(), DbError> {
        todo!()
    }

    pub fn set_excluded(&self, owner: &NoteOwner, model: &str, part_hash: &str, excluded: bool) -> Result<(), DbError> {
        todo!()
    }

    /// Явный сброс или удаление владельца. Пока заметки пишет только описание модели, удалять нечего,
    /// поэтому удаление персонажа и термина уже зовёт этот метод.
    pub fn delete_owner(&self, owner: &NoteOwner) -> Result<usize, DbError> {
        Ok(self.conn.execute("DELETE FROM notes WHERE owner_kind = ?1 AND owner_ref = ?2", params![owner.kind(), owner.reference()])?)
    }

    /// После повторного считывания: удаляет заметки, чьих частей нет среди всех частей владельца.
    pub fn retain_parts(&self, owner: &NoteOwner, part_hashes: &[String]) -> Result<usize, DbError> {
        todo!()
    }
}

/// Реплика текущего снимка для образца в запросе модели.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SampleLine {
    /// Подпись говорящего, у говорящего без подписи хвост ключа.
    pub speaker: Option<String>,
    pub text: String,
}

/// Реплики текущего снимка в порядке сюжета: образцы для предложений модели.
pub struct LineSampleRepo<'c> {
    pub conn: &'c Connection,
}

impl LineSampleRepo<'_> {
    const SELECT: &'static str = "SELECT sp.label, s.speaker_key, s.source_text
         FROM segments s
         LEFT JOIN containers c ON c.id = s.container_id
         LEFT JOIN speakers sp ON sp.key = s.speaker_key
         WHERE s.missing = 0";
    const ORDER: &'static str = "ORDER BY c.story_order IS NULL, c.story_order, s.story_order IS NULL, s.story_order, s.id";

    /// Все реплики говорящего.
    pub fn of_speaker(&self, key: &SpeakerKey) -> Result<Vec<SampleLine>, DbError> {
        self.query(&format!("{} AND s.speaker_key = ?1 {}", Self::SELECT, Self::ORDER), key.as_str())
    }

    /// Реплики, где есть `text`, без учёта регистра латиницы. Границы слов проверяет сервис. `None`: все.
    pub fn containing(&self, text: &str, limit: Option<usize>) -> Result<Vec<SampleLine>, DbError> {
        let limit = limit.map(|limit| format!(" LIMIT {limit}")).unwrap_or_default();
        self.query(&format!("{} AND instr(lower(s.source_text), lower(?1)) > 0 {}{limit}", Self::SELECT, Self::ORDER), text)
    }

    fn query(&self, sql: &str, value: &str) -> Result<Vec<SampleLine>, DbError> {
        let mut statement = self.conn.prepare(sql)?;
        let rows = statement
            .query_map([value], |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, String>(2)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows
            .into_iter()
            .map(|(label, key, text)| SampleLine {
                speaker: label.or_else(|| key.map(|key| key.split_once(':').map_or(key.as_str(), |(_, tail)| tail).to_string())),
                text,
            })
            .collect())
    }
}
