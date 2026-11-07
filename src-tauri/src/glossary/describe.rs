//! Запуски модели по глоссарию: описания по частям сцен, имена, токены, подписи говорящих, термины.

use tauri::ipc::Channel;

use super::{CharacterId, TermId};
use crate::common::AppError;
use crate::engines::ContainerKey;
use crate::llm::{ProfileId, RunProgress};
use crate::projects::ProjectId;
use crate::state::AppState;

/// Что описывается общим путём: части, заметки по частям, описание по заметкам.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DescribeTarget {
    Character(CharacterId),
    Term(TermId),
    /// Сцена: части это её реплики, заметка единственной части сразу становится описанием.
    Scene(ContainerKey),
}

/// Глубина чтения в массовом обновлении: минимум и максимум частей и желаемый процент.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadingDepth {
    pub min_parts: usize,
    pub max_parts: usize,
    pub percent: u8,
}

impl Default for ReadingDepth {
    fn default() -> Self {
        Self { min_parts: 5, max_parts: 50, percent: 30 }
    }
}

impl ReadingDepth {
    /// Процент от всех частей округляется вверх и ограничивается минимумом и максимумом.
    /// У кого частей меньше минимума, тот читается целиком (`partsForDepth`).
    pub fn parts_for(&self, total: usize) -> usize {
        if total <= self.min_parts {
            return total;
        }
        let wanted = (total * usize::from(self.percent)).div_ceil(100);
        wanted.clamp(self.min_parts, self.max_parts.max(self.min_parts)).min(total)
    }
}

/// Описания персонажей, терминов и сцен.
pub struct DescribeService<'a> {
    pub state: &'a AppState,
}

impl DescribeService<'_> {
    /// Читает `parts` частей и собирает описание. Описание по готовым заметкам собирается и после остановки.
    pub async fn describe(
        &self,
        project: ProjectId,
        target: DescribeTarget,
        parts: usize,
        one_off: Option<ProfileId>,
        progress: Channel<RunProgress>,
    ) -> Result<(), AppError> {
        todo!()
    }

    /// Группа по очереди с глубиной чтения. Заполненная запись без новых частей в запуск не входит.
    pub async fn describe_many(
        &self,
        project: ProjectId,
        targets: Vec<DescribeTarget>,
        depth: ReadingDepth,
        one_off: Option<ProfileId>,
        progress: Channel<RunProgress>,
    ) -> Result<(), AppError> {
        todo!()
    }

    /// Сбрасывает заметки владельца.
    pub fn reset_notes(&self, project: ProjectId, target: &DescribeTarget) -> Result<usize, AppError> {
        todo!()
    }
}

/// Имена и другие запуски по записям глоссария.
pub struct NamesService<'a> {
    pub state: &'a AppState,
}

impl NamesService<'_> {
    /// Имена всей группой в два шага: имена и фамилии, потом остальные формы. Возвращает предложения для отбора.
    pub async fn translate_names(
        &self,
        project: ProjectId,
        characters: Vec<CharacterId>,
        one_off: Option<ProfileId>,
        progress: Channel<RunProgress>,
    ) -> Result<serde_json::Value, AppError> {
        todo!()
    }

    pub async fn translate_terms(
        &self,
        project: ProjectId,
        terms: Vec<TermId>,
        one_off: Option<ProfileId>,
        progress: Channel<RunProgress>,
    ) -> Result<serde_json::Value, AppError> {
        todo!()
    }

    /// Пояснения токенов по репликам с `[___]` вместо токена.
    pub async fn suggest_tokens(&self, project: ProjectId, one_off: Option<ProfileId>, progress: Channel<RunProgress>) -> Result<serde_json::Value, AppError> {
        todo!()
    }

    /// Перевод подписи и род говорящих по их репликам.
    pub async fn suggest_speakers(&self, project: ProjectId, one_off: Option<ProfileId>, progress: Channel<RunProgress>) -> Result<serde_json::Value, AppError> {
        todo!()
    }

    /// Запрос для чат-бота без API: правила и данные одним текстом.
    pub fn chat_request(&self, project: ProjectId, characters: &[CharacterId]) -> Result<String, AppError> {
        todo!()
    }

    /// Ответ чат-бота проходит те же проверки и попадает в тот же отбор.
    pub fn read_chat_answer(&self, project: ProjectId, answer: &str) -> Result<serde_json::Value, AppError> {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::ReadingDepth;

    #[test]
    fn depth_rounds_up_and_clamps() {
        let depth = ReadingDepth::default();
        assert_eq!(depth.parts_for(3), 3);
        assert_eq!(depth.parts_for(10), 5);
        assert_eq!(depth.parts_for(21), 7);
        assert_eq!(depth.parts_for(1000), 50);
    }
}
