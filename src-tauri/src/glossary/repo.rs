//! Таблицы глоссария в project.db.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, Row, params};

use super::parts::scene_hash;
use super::scenes::SceneSource;
use super::{
    Character, CharacterId, DiscardReason, DiscardedTerm, Form, NameField, NameKey, NameOrigin, Note, NoteOwner, SceneDescription, Speaker,
    SpeakerEntry, SpeakerLink, Term, TermCategory, TermId, Token,
};
use crate::common::{Gender, Timestamp, ValidationError};
use crate::name_rules::LanguageTag;
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
    /// Сброс проекта: связи говорящих с персонажами, записи говорящих и корзина говорящих.
    pub fn clear(&self) -> Result<(), DbError> {
        self.conn.execute_batch("DELETE FROM speaker_characters; DELETE FROM speaker_entries; DELETE FROM discarded_speakers;")?;
        Ok(())
    }

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
    /// Сброс проекта: все персонажи, их формы, происхождение имён и исключённые сцены уходят каскадом.
    pub fn clear(&self) -> Result<(), DbError> {
        self.conn.execute("DELETE FROM characters", [])?;
        Ok(())
    }

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
        Ok(self.origins_of(Some(id))?.remove(&id).unwrap_or_default())
    }

    /// Происхождение имён всех персонажей.
    pub fn all_origins(&self) -> Result<HashMap<CharacterId, Vec<NameOrigin>>, DbError> {
        self.origins_of(None)
    }

    fn origins_of(&self, only: Option<CharacterId>) -> Result<HashMap<CharacterId, Vec<NameOrigin>>, DbError> {
        let mut statement = self.conn.prepare(
            "SELECT character_id, field, source, context, languages, reason FROM name_origins
             WHERE ?1 IS NULL OR character_id = ?1 ORDER BY character_id, field, source",
        )?;
        let rows = statement
            .query_map([only.map(CharacterId::get)], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut origins: HashMap<CharacterId, Vec<NameOrigin>> = HashMap::new();
        for (id, field, source, context, languages, reason) in rows {
            let field = NameField::parse(&field).map_err(|error| DbError::bad_value("name_origins", "field", error))?;
            let codes: Vec<String> = serde_json::from_str(&languages)
                .map_err(|_| DbError::bad_value("name_origins", "languages", ValidationError::BadLanguage(languages.clone())))?;
            let languages = codes
                .iter()
                .map(|code| LanguageTag::parse(code).map_err(|error| DbError::bad_value("name_origins", "languages", error)))
                .collect::<Result<Vec<_>, _>>()?;
            origins.entry(CharacterId::from_row(id)).or_default().push(NameOrigin { field, source, context, languages, reason });
        }
        Ok(origins)
    }

    /// Вставляет происхождение или заменяет прежнее того же поля и написания.
    pub fn save_origins(&self, id: CharacterId, origins: &[NameOrigin]) -> Result<(), DbError> {
        for origin in origins {
            let languages = serde_json::to_string(&origin.languages.iter().map(LanguageTag::as_str).collect::<Vec<_>>())
                .map_err(|error| DbError::Corrupt(error.to_string()))?;
            self.conn.execute(
                "INSERT INTO name_origins (character_id, field, source, context, languages, reason) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (character_id, field, source) DO UPDATE
                 SET context = excluded.context, languages = excluded.languages, reason = excluded.reason",
                params![id.get(), origin.field.as_str(), origin.source, origin.context, languages, origin.reason],
            )?;
        }
        Ok(())
    }

    pub fn excluded_containers(&self, id: CharacterId) -> Result<Vec<ContainerKey>, DbError> {
        let mut statement = self.conn.prepare("SELECT container_key FROM character_excluded_containers WHERE character_id = ?1 ORDER BY container_key")?;
        let keys = statement.query_map([id.get()], |row| row.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
        keys.into_iter().map(|key| ContainerKey::new(key).map_err(|error| DbError::bad_value("character_excluded_containers", "container_key", error))).collect()
    }

    pub fn set_excluded(&self, id: CharacterId, container: &ContainerKey, excluded: bool) -> Result<(), DbError> {
        if excluded {
            self.conn.execute("INSERT OR IGNORE INTO character_excluded_containers (character_id, container_key) VALUES (?1, ?2)", params![id.get(), container.as_str()])?;
        } else {
            self.conn.execute("DELETE FROM character_excluded_containers WHERE character_id = ?1 AND container_key = ?2", params![id.get(), container.as_str()])?;
        }
        Ok(())
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

/// `terms`, `term_forms`, `term_excluded_containers`, `discarded_terms`.
pub struct TermRepo<'c> {
    pub conn: &'c Connection,
}

const TERM_SELECT: &str = "SELECT id, name, translation, category, description FROM terms";

type TermRow = (i64, String, Option<String>, String, Option<String>);

impl TermRepo<'_> {
    /// Сброс проекта: все термины с формами и исключёнными сценами и корзина терминов.
    pub fn clear(&self) -> Result<(), DbError> {
        self.conn.execute_batch("DELETE FROM terms; DELETE FROM discarded_terms;")?;
        Ok(())
    }

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
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
    }

    fn term((id, name, translation, category, description): TermRow, forms: &mut HashMap<i64, Vec<Form>>) -> Result<Term, DbError> {
        let category = TermCategory::parse(&category).map_err(|error| DbError::bad_value("terms", "category", error))?;
        let forms = forms.remove(&id).unwrap_or_default();
        Term::restore(TermId::from_row(id), &name, translation, category, description, forms)
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
                    "UPDATE terms SET name = ?1, name_key = ?2, translation = ?3, category = ?4, description = ?5
                     WHERE id = ?6",
                    params![
                        term.name(),
                        term.name_key().as_str(),
                        term.translation(),
                        term.category().as_str(),
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
                    "INSERT INTO terms (name, name_key, translation, category, description) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        term.name(),
                        term.name_key().as_str(),
                        term.translation(),
                        term.category().as_str(),
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

    /// Есть ли название среди терминов, их форм, имён глоссария и корзины: так разбор сцены отсеивает известное.
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
        let mut statement = self.conn.prepare("SELECT container_key FROM term_excluded_containers WHERE term_id = ?1 ORDER BY container_key")?;
        let keys = statement.query_map([id.get()], |row| row.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
        keys.into_iter().map(|key| ContainerKey::new(key).map_err(|error| DbError::bad_value("term_excluded_containers", "container_key", error))).collect()
    }

    pub fn set_excluded(&self, id: TermId, container: &ContainerKey, excluded: bool) -> Result<(), DbError> {
        if excluded {
            self.conn.execute("INSERT OR IGNORE INTO term_excluded_containers (term_id, container_key) VALUES (?1, ?2)", params![id.get(), container.as_str()])?;
        } else {
            self.conn.execute("DELETE FROM term_excluded_containers WHERE term_id = ?1 AND container_key = ?2", params![id.get(), container.as_str()])?;
        }
        Ok(())
    }
}

/// `tokens`.
pub struct TokenRepo<'c> {
    pub conn: &'c Connection,
}

impl TokenRepo<'_> {
    /// Сброс проекта: значения и пояснения всех токенов.
    pub fn clear(&self) -> Result<(), DbError> {
        self.conn.execute("DELETE FROM tokens", [])?;
        Ok(())
    }

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

/// `reading_depth` в app.db: одна строка на приложение.
pub struct ReadingDepthRepo<'c> {
    pub conn: &'c Connection,
}

impl ReadingDepthRepo<'_> {
    /// Без строки действует глубина из кода. `max_parts` `NULL`: без предела.
    pub fn get(&self) -> Result<super::ReadingDepth, DbError> {
        let row = self
            .conn
            .query_row("SELECT min_parts, max_parts, percent FROM reading_depth WHERE id = 1", [], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, Option<i64>>(1)?, row.get::<_, i64>(2)?))
            })
            .optional()?;
        let Some((min, max, percent)) = row else { return Ok(super::ReadingDepth::default()) };
        let bad = |field: &'static str| DbError::bad_value("reading_depth", field, ValidationError::OutOfRange(field));
        let min = usize::try_from(min).map_err(|_| bad("min_parts"))?;
        let max = max.map(usize::try_from).transpose().map_err(|_| bad("max_parts"))?;
        let percent = u8::try_from(percent).map_err(|_| bad("percent"))?;
        super::ReadingDepth::new(min, max, percent).map_err(|error| DbError::bad_value("reading_depth", "min_parts", error))
    }

    pub fn save(&self, depth: &super::ReadingDepth) -> Result<(), DbError> {
        let to_sql = |value: usize| i64::try_from(value).unwrap_or(i64::MAX);
        self.conn.execute(
            "INSERT INTO reading_depth (id, min_parts, max_parts, percent) VALUES (1, ?1, ?2, ?3)
             ON CONFLICT (id) DO UPDATE SET min_parts = excluded.min_parts, max_parts = excluded.max_parts, percent = excluded.percent",
            params![to_sql(depth.min_parts()), depth.max_parts().map(to_sql), i64::from(depth.percent())],
        )?;
        Ok(())
    }
}

/// `scene_descriptions`.
pub struct SceneDescriptionRepo<'c> {
    pub conn: &'c Connection,
}

impl SceneDescriptionRepo<'_> {
    /// Сброс проекта: названия и описания всех сцен.
    pub fn clear(&self) -> Result<(), DbError> {
        self.conn.execute("DELETE FROM scene_descriptions", [])?;
        Ok(())
    }

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

    /// Сцены текущего снимка с репликами в порядке сюжета. Отпечаток текста считается по отпечаткам оригиналов.
    pub fn sources(&self) -> Result<Vec<SceneSource>, DbError> {
        let mut statement = self.conn.prepare(
            "SELECT c.key, f.path, s.source_hash
             FROM containers c
             JOIN segments s ON s.container_id = c.id AND s.missing = 0
             LEFT JOIN files f ON f.id = s.file_id
             WHERE c.kind = 'scene' AND c.last_seen_scan = (SELECT max(id) FROM scans)
             ORDER BY c.story_order IS NULL, c.story_order, c.key, s.story_order, s.id",
        )?;
        let rows = statement
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, String>(2)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        let mut grouped: Vec<(String, Option<String>, Vec<String>)> = Vec::new();
        for (key, file, hash) in rows {
            match grouped.last_mut() {
                Some((last, _, hashes)) if *last == key => hashes.push(hash),
                _ => grouped.push((key, file, vec![hash])),
            }
        }
        grouped
            .into_iter()
            .map(|(key, file, hashes)| {
                Ok(SceneSource {
                    key: ContainerKey::new(key).map_err(|error| DbError::bad_value("containers", "key", error))?,
                    file,
                    lines: hashes.len(),
                    text_hash: scene_hash(hashes.iter().map(String::as_str)),
                })
            })
            .collect()
    }

    /// Первая сцена игры по сюжету: модель узнаёт, что это вступление.
    pub fn first_scene(&self) -> Result<Option<ContainerKey>, DbError> {
        let key: Option<String> = self
            .conn
            .query_row(
                "SELECT key FROM containers WHERE kind = 'scene' AND story_order IS NOT NULL
                   AND last_seen_scan = (SELECT max(id) FROM scans)
                 ORDER BY story_order LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        key.map(ContainerKey::new).transpose().map_err(|error| DbError::bad_value("containers", "key", error))
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
    /// Сброс проекта: все заметки модели.
    pub fn clear(&self) -> Result<(), DbError> {
        self.conn.execute("DELETE FROM notes", [])?;
        Ok(())
    }

    /// Заметки владельца от одной модели.
    pub fn of(&self, owner: &NoteOwner, model: &str) -> Result<Vec<Note>, DbError> {
        self.query(
            "SELECT model, part_hash, text, excluded, created_at FROM notes
             WHERE owner_kind = ?1 AND owner_ref = ?2 AND model = ?3 ORDER BY created_at, part_hash",
            owner,
            Some(model),
        )
    }

    /// Заметки владельца от всех моделей, новые первыми.
    pub fn all_of(&self, owner: &NoteOwner) -> Result<Vec<Note>, DbError> {
        self.query(
            "SELECT model, part_hash, text, excluded, created_at FROM notes
             WHERE owner_kind = ?1 AND owner_ref = ?2 ORDER BY created_at DESC, model",
            owner,
            None,
        )
    }

    fn query(&self, sql: &str, owner: &NoteOwner, model: Option<&str>) -> Result<Vec<Note>, DbError> {
        let mut statement = self.conn.prepare(sql)?;
        let reference = owner.reference();
        let read = |row: &Row| {
            Ok(Note {
                owner: owner.clone(),
                model: row.get(0)?,
                part_hash: row.get(1)?,
                text: row.get(2)?,
                excluded: row.get::<_, i64>(3)? != 0,
                created_at: Timestamp::from_secs(row.get(4)?),
            })
        };
        let notes = match model {
            Some(model) => statement.query_map(params![owner.kind(), reference, model], read)?.collect::<Result<Vec<_>, _>>()?,
            None => statement.query_map(params![owner.kind(), reference], read)?.collect::<Result<Vec<_>, _>>()?,
        };
        Ok(notes)
    }

    /// Новая заметка той же части заменяет прежнюю.
    pub fn save(&self, note: &Note) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT INTO notes (owner_kind, owner_ref, model, part_hash, text, excluded, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (owner_kind, owner_ref, model, part_hash) DO UPDATE SET text = excluded.text,
               excluded = excluded.excluded, created_at = excluded.created_at",
            params![
                note.owner.kind(),
                note.owner.reference(),
                note.model,
                note.part_hash,
                note.text,
                i64::from(note.excluded),
                note.created_at.secs()
            ],
        )?;
        Ok(())
    }

    pub fn set_excluded(&self, owner: &NoteOwner, model: &str, part_hash: &str, excluded: bool) -> Result<(), DbError> {
        self.conn.execute(
            "UPDATE notes SET excluded = ?5 WHERE owner_kind = ?1 AND owner_ref = ?2 AND model = ?3 AND part_hash = ?4",
            params![owner.kind(), owner.reference(), model, part_hash, i64::from(excluded)],
        )?;
        Ok(())
    }

    /// Число заметок по владельцам одного вида: окно показывает, у кого они есть.
    pub fn counts(&self, kind: &str) -> Result<HashMap<String, usize>, DbError> {
        let mut statement = self.conn.prepare("SELECT owner_ref, count(*) FROM notes WHERE owner_kind = ?1 GROUP BY owner_ref")?;
        let rows = statement.query_map([kind], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))?;
        let mut counts = HashMap::new();
        for row in rows {
            let (owner, count) = row?;
            counts.insert(owner, usize::try_from(count).unwrap_or(0));
        }
        Ok(counts)
    }

    /// Явный сброс или удаление владельца. Пока заметки пишет только описание модели, удалять нечего,
    /// поэтому удаление персонажа и термина уже зовёт этот метод.
    pub fn delete_owner(&self, owner: &NoteOwner) -> Result<usize, DbError> {
        Ok(self.conn.execute("DELETE FROM notes WHERE owner_kind = ?1 AND owner_ref = ?2", params![owner.kind(), owner.reference()])?)
    }

    /// Все владельцы, у которых есть заметки.
    pub fn owners(&self) -> Result<Vec<NoteOwner>, DbError> {
        let mut statement = self.conn.prepare("SELECT DISTINCT owner_kind, owner_ref FROM notes ORDER BY owner_kind, owner_ref")?;
        let rows = statement
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        rows.iter()
            .map(|(kind, reference)| NoteOwner::parse(kind, reference).map_err(|error| DbError::bad_value("notes", "owner_ref", error)))
            .collect()
    }

    /// После повторного считывания: удаляет заметки, чьих частей нет среди всех частей владельца.
    pub fn retain_parts(&self, owner: &NoteOwner, part_hashes: &[String]) -> Result<usize, DbError> {
        let mut statement = self.conn.prepare("SELECT DISTINCT part_hash FROM notes WHERE owner_kind = ?1 AND owner_ref = ?2")?;
        let stored = statement
            .query_map(params![owner.kind(), owner.reference()], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let mut removed = 0;
        for hash in stored.iter().filter(|hash| !part_hashes.contains(hash)) {
            removed += self.conn.execute(
                "DELETE FROM notes WHERE owner_kind = ?1 AND owner_ref = ?2 AND part_hash = ?3",
                params![owner.kind(), owner.reference(), hash],
            )?;
        }
        Ok(removed)
    }
}

/// Реплика текущего снимка для образца в запросе модели.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SampleLine {
    /// Подпись говорящего, у говорящего без подписи хвост ключа.
    pub speaker: Option<String>,
    pub text: String,
}

/// Реплика сцены текущего снимка для описаний персонажей и терминов.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoryLine {
    pub scene: ContainerKey,
    pub speaker_key: Option<SpeakerKey>,
    /// Подпись говорящего из игры.
    pub label: Option<String>,
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

    /// Реплики всех сцен текущего снимка по сюжету: из них выбираются сцены персонажа и термина.
    pub fn story_lines(&self) -> Result<Vec<StoryLine>, DbError> {
        self.lines_of(None)
    }

    /// Реплики одной сцены по сюжету: по ним ищутся её персонажи и термины.
    pub fn scene_lines(&self, scene: &ContainerKey) -> Result<Vec<StoryLine>, DbError> {
        self.lines_of(Some(scene))
    }

    fn lines_of(&self, scene: Option<&ContainerKey>) -> Result<Vec<StoryLine>, DbError> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT c.key, s.speaker_key, sp.label, s.source_text
             FROM segments s
             JOIN containers c ON c.id = s.container_id AND c.kind = 'scene'
             LEFT JOIN speakers sp ON sp.key = s.speaker_key
             WHERE s.missing = 0 AND (?1 IS NULL OR c.key = ?1) {}",
            Self::ORDER
        ))?;
        let rows = statement
            .query_map([scene.map(ContainerKey::as_str)], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, String>(3)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(scene, speaker, label, text)| {
                Ok(StoryLine {
                    scene: ContainerKey::new(scene).map_err(|error| DbError::bad_value("containers", "key", error))?,
                    speaker_key: speaker.map(|key| speaker_key("segments", key)).transpose()?,
                    label,
                    text,
                })
            })
            .collect()
    }

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

/// `analyzed_parts`: части сцен, которые прочитал разбор.
pub struct AnalyzedPartRepo<'c> {
    pub conn: &'c Connection,
}

impl AnalyzedPartRepo<'_> {
    /// Сброс проекта: разбор прочитает все части заново.
    pub fn clear(&self) -> Result<(), DbError> {
        self.conn.execute("DELETE FROM analyzed_parts", [])?;
        Ok(())
    }

    /// Отпечатки прочитанных разбором частей сцены.
    pub fn of(&self, key: &ContainerKey) -> Result<std::collections::HashSet<String>, DbError> {
        let mut statement = self.conn.prepare("SELECT part_hash FROM analyzed_parts WHERE container_key = ?1")?;
        let rows = statement.query_map([key.as_str()], |row| row.get::<_, String>(0))?.collect::<Result<_, _>>()?;
        Ok(rows)
    }

    pub fn mark(&self, key: &ContainerKey, part_hash: &str, now: Timestamp) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT OR IGNORE INTO analyzed_parts (container_key, part_hash, analyzed_at) VALUES (?1, ?2, ?3)",
            params![key.as_str(), part_hash, now.secs()],
        )?;
        Ok(())
    }

    /// Сцены с отметками: по ним идёт уборка после считывания.
    pub fn scenes(&self) -> Result<Vec<ContainerKey>, DbError> {
        let mut statement = self.conn.prepare("SELECT DISTINCT container_key FROM analyzed_parts ORDER BY container_key")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
        rows.into_iter().map(|key| ContainerKey::new(key).map_err(|error| DbError::bad_value("analyzed_parts", "container_key", error))).collect()
    }

    /// Удаляет отметки частей, которых больше нет у сцены. Возвращает число удалённых.
    pub fn retain(&self, key: &ContainerKey, part_hashes: &[String]) -> Result<usize, DbError> {
        let mut removed = 0;
        for hash in self.of(key)?.iter().filter(|hash| !part_hashes.contains(hash)) {
            removed += self
                .conn
                .execute("DELETE FROM analyzed_parts WHERE container_key = ?1 AND part_hash = ?2", params![key.as_str(), hash])?;
        }
        Ok(removed)
    }
}
