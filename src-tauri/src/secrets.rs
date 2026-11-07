//! Ключи API провайдеров. Только этот модуль знает, где они лежат.

use std::collections::HashMap;

use parking_lot::Mutex;

use super::llm::{ApiKey, ProviderId};

/// Имя записи в хранилище учётных данных: `TranslatorApp/llm/<id провайдера>`.
const SERVICE: &str = "TranslatorApp";

/// Ключи API. В базе их нет: файл базы легко скопировать или отправить с логами.
pub enum SecretStore {
    /// Системное хранилище через `keyring`: Credential Manager, Keychain, Secret Service.
    System,
    /// Хранилища нет: ключ живёт в памяти до закрытия приложения и на диск открытым текстом не пишется.
    Memory(Mutex<HashMap<ProviderId, ApiKey>>),
}

impl SecretStore {
    /// Системное хранилище, если оно доступно, иначе память.
    pub fn detect() -> Self {
        todo!()
    }

    pub fn get(&self, provider: ProviderId) -> Option<ApiKey> {
        match self {
            Self::System => todo!(),
            Self::Memory(keys) => keys.lock().get(&provider).cloned(),
        }
    }

    pub fn set(&self, provider: ProviderId, key: ApiKey) {
        match self {
            Self::System => todo!(),
            Self::Memory(keys) => {
                keys.lock().insert(provider, key);
            }
        }
    }

    pub fn delete(&self, provider: ProviderId) {
        match self {
            Self::System => todo!(),
            Self::Memory(keys) => {
                keys.lock().remove(&provider);
            }
        }
    }

    /// Запуск приложения: удаляет ключи провайдеров, которых нет в базе (сбой между базой и хранилищем).
    pub fn delete_except(&self, providers: &[ProviderId]) {
        match self {
            // Хранилище ОС не перечисляет записи: список ключей появится вместе с поддержкой keyring.
            Self::System => todo!(),
            Self::Memory(keys) => keys.lock().retain(|id, _| providers.contains(id)),
        }
    }

    fn entry_name(provider: ProviderId) -> String {
        format!("llm/{provider}")
    }
}
