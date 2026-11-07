//! Сборка и установка мода. Подписи говорящих переводит сервис, движок только кладёт пары в мод.

use std::path::PathBuf;

use crate::common::AppError;
use crate::engines::ModInput;
use crate::glossary::{Character, SpeakerEntry, Term};
use crate::projects::ProjectId;
use crate::state::AppState;

/// Подпись говорящего текущего снимка и что о ней известно из глоссария.
#[derive(Debug, Clone)]
pub struct SpeakerLabelSource<'a> {
    pub label: &'a str,
    pub entry: Option<&'a SpeakerEntry>,
    /// Связанные персонажи.
    pub linked: Vec<&'a Character>,
}

/// Перевод подписей говорящих (`character_name_strings` в 1.0). Первый найденный побеждает:
/// 1. перевод подписи в записи говорящего;
/// 2. имя связанного персонажа, подпись на нескольких переводится по частям;
/// 3. совпавшее имя любого персонажа;
/// 4. подтверждённый термин.
///
/// Подпись из одной переменной не переводится, несколько разных переводов на одном шаге пропускают подпись.
pub struct SpeakerLabels;

impl SpeakerLabels {
    pub fn translate(labels: &[SpeakerLabelSource<'_>], characters: &[Character], terms: &[Term]) -> Vec<(String, String)> {
        todo!()
    }
}

pub struct ExportService<'a> {
    pub state: &'a AppState,
}

impl ExportService<'_> {
    /// Всё для мода из project.db: текущий снимок, действующие переводы, подписи говорящих, шрифты.
    pub fn mod_input(&self, project: ProjectId) -> Result<ModInput, AppError> {
        todo!()
    }

    /// Собирает мод в `exports/`. Папка игры не нужна.
    pub fn build(&self, project: ProjectId) -> Result<PathBuf, AppError> {
        todo!()
    }

    /// Ставит мод в привязанную папку: нужна привязка (`is_bound`) и папка на диске.
    pub fn install(&self, project: ProjectId) -> Result<(), AppError> {
        todo!()
    }

    pub fn uninstall(&self, project: ProjectId) -> Result<(), AppError> {
        todo!()
    }
}
