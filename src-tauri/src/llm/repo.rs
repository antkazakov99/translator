//! Таблицы LLM: `llm_providers`, `llm_profiles` в app.db и `llm_action_settings` в обеих базах.

use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{Action, ActionSettings, Profile, ProfileId, Provider, ProviderId, ProviderKind};
use crate::storage::DbError;

pub struct ProviderRepo<'c> {
    pub conn: &'c Connection,
}

impl ProviderRepo<'_> {
    pub fn get(&self, id: ProviderId) -> Result<Provider, DbError> {
        self.conn
            .query_row("SELECT id, position, name, kind, url FROM llm_providers WHERE id = ?1", [id.to_string()], read_provider)
            .optional()?
            .ok_or(DbError::NotFound)?
    }

    /// По `position`.
    pub fn list(&self) -> Result<Vec<Provider>, DbError> {
        let mut statement = self.conn.prepare("SELECT id, position, name, kind, url FROM llm_providers ORDER BY position, name")?;
        let rows = statement.query_map([], read_provider)?.collect::<Result<Vec<_>, _>>()?;
        rows.into_iter().collect()
    }

    pub fn save(&self, provider: &Provider) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT INTO llm_providers (id, position, name, kind, url) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (id) DO UPDATE SET
               position = excluded.position, name = excluded.name, kind = excluded.kind, url = excluded.url",
            params![
                provider.id().to_string(),
                provider.position(),
                provider.name(),
                provider.kind().as_str(),
                provider.url().as_str()
            ],
        )?;
        Ok(())
    }

    /// Профили удаляются каскадом, у действий профиль обнуляется.
    pub fn delete(&self, id: ProviderId) -> Result<(), DbError> {
        self.conn.execute("DELETE FROM llm_providers WHERE id = ?1", [id.to_string()])?;
        Ok(())
    }
}

/// Строка в `Provider`. Внешний `Result` это ошибка SQLite, внутренний неверное значение в строке.
fn read_provider(row: &Row) -> rusqlite::Result<Result<Provider, DbError>> {
    let id: String = row.get(0)?;
    let position: u32 = row.get(1)?;
    let name: String = row.get(2)?;
    let kind: String = row.get(3)?;
    let url: String = row.get(4)?;
    Ok((|| {
        let bad = |column| move |error| DbError::bad_value("llm_providers", column, error);
        let id = ProviderId::parse(&id).map_err(bad("id"))?;
        let kind = ProviderKind::parse(&kind).map_err(bad("kind"))?;
        Provider::restore(id, position, &name, kind, &url).map_err(bad("url"))
    })())
}

pub struct ProfileRepo<'c> {
    pub conn: &'c Connection,
}

impl ProfileRepo<'_> {
    pub fn get(&self, id: ProfileId) -> Result<Profile, DbError> {
        todo!()
    }

    /// Общий список профилей по `position`.
    pub fn list(&self) -> Result<Vec<Profile>, DbError> {
        todo!()
    }

    /// Профиль действий, у которых свой не задан.
    pub fn first(&self) -> Result<Option<Profile>, DbError> {
        todo!()
    }

    pub fn save(&self, profile: &Profile) -> Result<(), DbError> {
        todo!()
    }

    pub fn delete(&self, id: ProfileId) -> Result<(), DbError> {
        todo!()
    }
}

/// `llm_action_settings`. Таблица одинаковая в app.db и project.db, поэтому репозиторий один на обе базы.
/// В проекте внешних ключей нет: неизвестное действие пропускается, неизвестный профиль проверяет `LlmTargetService`.
pub struct ActionRepo<'c> {
    pub conn: &'c Connection,
}

impl ActionRepo<'_> {
    /// Нет строки: все поля пустые.
    pub fn settings(&self, action: Action) -> Result<ActionSettings, DbError> {
        todo!()
    }

    /// Все строки с известными действиями.
    pub fn all(&self) -> Result<Vec<(Action, ActionSettings)>, DbError> {
        todo!()
    }

    /// Все поля пустые: строка удаляется.
    pub fn save(&self, action: Action, settings: &ActionSettings) -> Result<(), DbError> {
        todo!()
    }
}
