//! Таблицы LLM: `llm_providers`, `llm_profiles` в app.db и `llm_action_settings` в обеих базах.

use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{Action, ActionSettings, BatchSize, Params, Profile, ProfileId, Provider, ProviderId, ProviderKind, Temperature};
use crate::common::ValidationError;
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

const PROFILE_COLUMNS: &str = "id, provider_id, position, name, model, context_tokens, reasoning_tokens, timeout_secs, params";

impl ProfileRepo<'_> {
    pub fn get(&self, id: ProfileId) -> Result<Profile, DbError> {
        self.conn
            .query_row(&format!("SELECT {PROFILE_COLUMNS} FROM llm_profiles WHERE id = ?1"), [id.to_string()], read_profile)
            .optional()?
            .ok_or(DbError::NotFound)?
    }

    /// Общий список профилей по `position`.
    pub fn list(&self) -> Result<Vec<Profile>, DbError> {
        let mut statement = self.conn.prepare(&format!("SELECT {PROFILE_COLUMNS} FROM llm_profiles ORDER BY position, name"))?;
        let rows = statement.query_map([], read_profile)?.collect::<Result<Vec<_>, _>>()?;
        rows.into_iter().collect()
    }

    /// Профиль действий, у которых свой не задан.
    pub fn first(&self) -> Result<Option<Profile>, DbError> {
        self.conn
            .query_row(&format!("SELECT {PROFILE_COLUMNS} FROM llm_profiles ORDER BY position, name LIMIT 1"), [], read_profile)
            .optional()?
            .transpose()
    }

    pub fn save(&self, profile: &Profile) -> Result<(), DbError> {
        let fields = profile.params().fields();
        let params = (!fields.is_empty()).then(|| serde_json::Value::Object(fields.clone()).to_string());
        self.conn.execute(
            "INSERT INTO llm_profiles (id, provider_id, position, name, model, context_tokens, reasoning_tokens, timeout_secs, params)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT (id) DO UPDATE SET
               provider_id = excluded.provider_id, position = excluded.position, name = excluded.name,
               model = excluded.model, context_tokens = excluded.context_tokens,
               reasoning_tokens = excluded.reasoning_tokens, timeout_secs = excluded.timeout_secs, params = excluded.params",
            params![
                profile.id().to_string(),
                profile.provider_id().to_string(),
                profile.position(),
                profile.name(),
                profile.model(),
                profile.context_tokens(),
                profile.reasoning_limit(),
                profile.timeout_limit(),
                params
            ],
        )?;
        Ok(())
    }

    /// Только место в общем списке.
    pub fn set_position(&self, id: ProfileId, position: u32) -> Result<(), DbError> {
        self.conn.execute("UPDATE llm_profiles SET position = ?2 WHERE id = ?1", params![id.to_string(), position])?;
        Ok(())
    }

    /// У действий приложения профиль обнуляется. Переопределения в проектах проверяет `LlmTargetService`.
    pub fn delete(&self, id: ProfileId) -> Result<(), DbError> {
        self.conn.execute("DELETE FROM llm_profiles WHERE id = ?1", [id.to_string()])?;
        Ok(())
    }
}

fn read_profile(row: &Row) -> rusqlite::Result<Result<Profile, DbError>> {
    let id: String = row.get(0)?;
    let provider: String = row.get(1)?;
    let position: u32 = row.get(2)?;
    let name: String = row.get(3)?;
    let model: String = row.get(4)?;
    let context: Option<u32> = row.get(5)?;
    let reasoning: Option<u32> = row.get(6)?;
    let timeout: Option<u32> = row.get(7)?;
    let params: Option<String> = row.get(8)?;
    Ok((|| {
        let bad = |column| move |error| DbError::bad_value("llm_profiles", column, error);
        let id = ProfileId::parse(&id).map_err(bad("id"))?;
        let provider = ProviderId::parse(&provider).map_err(bad("provider_id"))?;
        let params = match params {
            None => Params::default(),
            Some(text) => match serde_json::from_str(&text) {
                Ok(serde_json::Value::Object(fields)) => Params::new(fields),
                _ => return Err(bad("params")(ValidationError::UnknownValue { field: "llm_profiles.params", value: text })),
            },
        };
        Profile::restore(id, provider, position, &name, &model, context, reasoning, timeout, params).map_err(bad("model"))
    })())
}

/// `llm_action_settings`. Таблица одинаковая в app.db и project.db, поэтому репозиторий один на обе базы.
/// В проекте внешних ключей нет: неизвестное действие пропускается, неизвестный профиль проверяет `LlmTargetService`.
pub struct ActionRepo<'c> {
    pub conn: &'c Connection,
}

impl ActionRepo<'_> {
    /// Нет строки: все поля пустые.
    pub fn settings(&self, action: Action) -> Result<ActionSettings, DbError> {
        let row = self
            .conn
            .query_row(
                "SELECT action_id, profile_id, temperature, batch_size FROM llm_action_settings WHERE action_id = ?1",
                [action.id()],
                read_action,
            )
            .optional()?;
        Ok(row.transpose()?.flatten().map(|(_, settings)| settings).unwrap_or_default())
    }

    /// Все строки с известными действиями.
    pub fn all(&self) -> Result<Vec<(Action, ActionSettings)>, DbError> {
        let mut statement = self.conn.prepare("SELECT action_id, profile_id, temperature, batch_size FROM llm_action_settings")?;
        let rows = statement.query_map([], read_action)?.collect::<Result<Vec<_>, _>>()?;
        let mut all = Vec::with_capacity(rows.len());
        for row in rows {
            all.extend(row?);
        }
        Ok(all)
    }

    /// Все поля пустые: строка удаляется.
    pub fn save(&self, action: Action, settings: &ActionSettings) -> Result<(), DbError> {
        if *settings == ActionSettings::default() {
            self.conn.execute("DELETE FROM llm_action_settings WHERE action_id = ?1", [action.id()])?;
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO llm_action_settings (action_id, profile_id, temperature, batch_size) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (action_id) DO UPDATE SET
               profile_id = excluded.profile_id, temperature = excluded.temperature, batch_size = excluded.batch_size",
            params![
                action.id(),
                settings.profile.map(|id| id.to_string()),
                settings.temperature.map(|value| value.as_f64()),
                settings.batch_size.map(|value| value.get() as i64)
            ],
        )?;
        Ok(())
    }
}

/// `None`: действие неизвестно этой версии.
fn read_action(row: &Row) -> rusqlite::Result<Result<Option<(Action, ActionSettings)>, DbError>> {
    let action: String = row.get(0)?;
    let profile: Option<String> = row.get(1)?;
    let temperature: Option<f64> = row.get(2)?;
    let batch_size: Option<i64> = row.get(3)?;
    Ok((|| {
        let Some(action) = Action::parse(&action) else { return Ok(None) };
        let bad = |column| move |error| DbError::bad_value("llm_action_settings", column, error);
        let settings = ActionSettings {
            profile: profile.as_deref().map(ProfileId::parse).transpose().map_err(bad("profile_id"))?,
            temperature: temperature.map(Temperature::new).transpose().map_err(bad("temperature"))?,
            batch_size: batch_size.map(BatchSize::new).transpose().map_err(bad("batch_size"))?,
        };
        Ok(Some((action, settings)))
    })())
}
