//! Ключи API провайдеров. Только этот модуль знает, где они лежат.
//!
//! Ключ никогда не переживает своего провайдера: при удалении сначала ключ, потом строка в базе,
//! при создании сначала строка, потом ключ. Сбой посередине оставляет провайдера без ключа,
//! а не ключ без провайдера, поэтому уборка ключей при запуске не нужна.

use std::collections::HashMap;

use parking_lot::Mutex;
use thiserror::Error;

use super::llm::{ApiKey, ProviderId};

/// Служба записей в хранилище учётных данных. Полное имя записи `TranslatorApp/llm/<id провайдера>`.
const SERVICE: &str = "TranslatorApp";

/// Ошибка хранилища ОС. Текст ошибки `keyring` секрета не содержит, отладочный вывод не используется.
#[derive(Debug, Error)]
#[error("Хранилище ключей недоступно: {0}")]
pub struct SecretError(String);

impl From<keyring::Error> for SecretError {
    fn from(error: keyring::Error) -> Self {
        Self(error.to_string())
    }
}

/// Ключи API. В базе их нет: файл базы легко скопировать или отправить с логами.
pub enum SecretStore {
    /// Хранилище ОС через `keyring`: Credential Manager, Keychain, Secret Service.
    System,
    /// Хранилища нет: ключ живёт в памяти до закрытия приложения и на диск открытым текстом не пишется.
    Memory(Mutex<HashMap<ProviderId, ApiKey>>),
}

impl SecretStore {
    /// Хранилище ОС, если оно доступно, иначе память.
    pub fn detect() -> Self {
        match keyring::Entry::store_status() {
            Ok(()) => Self::System,
            Err(error) => {
                tracing::warn!("Хранилище ключей ОС недоступно, ключи будут только в памяти: {error}");
                Self::memory()
            }
        }
    }

    pub fn memory() -> Self {
        Self::Memory(Mutex::new(HashMap::new()))
    }

    /// Переживёт ли ключ закрытие приложения: окно предупреждает, если нет.
    pub fn is_persistent(&self) -> bool {
        matches!(self, Self::System)
    }

    pub fn get(&self, provider: ProviderId) -> Result<Option<ApiKey>, SecretError> {
        match self {
            Self::System => match entry(provider)?.get_password() {
                Ok(text) => Ok(ApiKey::new(&text)),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(error) => Err(error.into()),
            },
            Self::Memory(keys) => Ok(keys.lock().get(&provider).cloned()),
        }
    }

    pub fn set(&self, provider: ProviderId, key: &ApiKey) -> Result<(), SecretError> {
        match self {
            Self::System => Ok(entry(provider)?.set_password(key.expose())?),
            Self::Memory(keys) => {
                keys.lock().insert(provider, key.clone());
                Ok(())
            }
        }
    }

    /// Удаляет ключ. Ключа и так нет: не ошибка.
    pub fn delete(&self, provider: ProviderId) -> Result<(), SecretError> {
        match self {
            Self::System => match entry(provider)?.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(error) => Err(error.into()),
            },
            Self::Memory(keys) => {
                keys.lock().remove(&provider);
                Ok(())
            }
        }
    }
}

fn entry(provider: ProviderId) -> Result<keyring::Entry, SecretError> {
    Ok(keyring::Entry::new(SERVICE, &format!("llm/{provider}"))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(store: &SecretStore) {
        let provider = ProviderId::generate();
        assert!(store.get(provider).unwrap().is_none());
        store.set(provider, &ApiKey::new("sk-тест").unwrap()).unwrap();
        assert_eq!(store.get(provider).unwrap().unwrap().expose(), "sk-тест");
        store.set(provider, &ApiKey::new("sk-новый").unwrap()).unwrap();
        assert_eq!(store.get(provider).unwrap().unwrap().expose(), "sk-новый");
        store.delete(provider).unwrap();
        assert!(store.get(provider).unwrap().is_none());
        store.delete(provider).unwrap();
    }

    #[test]
    fn memory_store_keeps_keys_until_deleted() {
        let store = SecretStore::memory();
        assert!(!store.is_persistent());
        round_trip(&store);
    }

    /// Пишет и удаляет тестовую запись в хранилище ОС: `cargo test --lib -- --ignored`.
    #[test]
    #[ignore = "Пишет запись в хранилище учётных данных ОС"]
    fn system_store_keeps_keys_until_deleted() {
        let store = SecretStore::detect();
        assert!(store.is_persistent(), "хранилище ОС недоступно");
        round_trip(&store);
    }
}
