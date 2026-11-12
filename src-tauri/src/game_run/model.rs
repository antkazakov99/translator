//! Шаги цепочки, их порядок, сводка и итог.

use serde::{Deserialize, Serialize};

use crate::glossary::NamesFound;

/// Шаг цепочки. Порядок вариантов это порядок выполнения.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum GameStep {
    /// Разбор сцен: новые персонажи и термины.
    Analysis,
    /// Имена персонажей: имена и фамилии, потом формы. Раньше остальных переводов: термин и подпись с именем
    /// берут готовое написание.
    Names,
    /// Переводы глоссария: термины, подписи говорящих, значения токенов.
    Glossary,
    /// Описания персонажей и терминов.
    Entries,
    /// Описания сцен.
    Scenes,
    /// Перевод реплик.
    Lines,
    /// Вычитка.
    Review,
}

impl GameStep {
    pub const ALL: [GameStep; 7] = [Self::Analysis, Self::Names, Self::Glossary, Self::Entries, Self::Scenes, Self::Lines, Self::Review];

    pub fn title(self) -> &'static str {
        match self {
            Self::Analysis => "Разбор сцен",
            Self::Names => "Перевод имён",
            Self::Glossary => "Переводы глоссария",
            Self::Entries => "Описания персонажей и терминов",
            Self::Scenes => "Описания сцен",
            Self::Lines => "Перевод реплик",
            Self::Review => "Вычитка",
        }
    }
}

/// Шаги в порядке выполнения без повторов.
pub fn ordered(steps: &[GameStep]) -> Vec<GameStep> {
    GameStep::ALL.into_iter().filter(|step| steps.contains(step)).collect()
}

/// Как цепочка выбирает написание имени.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum NamesMode {
    /// Написание, на котором перевод и проверка сошлись, пишется сразу. Спорные идут в отбор, как в ручном режиме.
    Auto,
    /// Все написания идут в отбор: цепочка останавливается на нём, окно продолжает её после записи.
    Manual,
}

/// Что осталось сделать шагу: «сцен без описания: 140».
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkCount {
    pub label: &'static str,
    pub count: usize,
}

/// Сводка шага для карточки.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepWork {
    pub step: GameStep,
    pub left: Vec<WorkCount>,
    /// Примерное число запросов к модели.
    pub requests: usize,
}

impl StepWork {
    /// Делать нечего: шаг недоступен.
    pub fn is_empty(&self) -> bool {
        self.left.iter().all(|count| count.count == 0)
    }
}

/// Шаг, который прошёл до конца, и сколько он записал.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepResult {
    pub step: GameStep,
    pub written: usize,
}

/// Итог цепочки.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GameRun {
    pub finished: Vec<StepResult>,
    /// Пользователь остановил цепочку.
    pub stopped: bool,
    /// Ошибка, которая оборвала цепочку, с названием шага.
    pub error: Option<String>,
    /// Написания имён для отбора: все в ручном режиме, спорные в «Авто». Цепочка встала, `rest` это шаги после
    /// отбора. Шаг имён стоит в них первым: после записи он спросит формы.
    pub names: Option<NamesFound>,
    pub rest: Vec<GameStep>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_run_in_fixed_order_once() {
        let steps = ordered(&[GameStep::Review, GameStep::Analysis, GameStep::Lines, GameStep::Analysis]);
        assert_eq!(steps, vec![GameStep::Analysis, GameStep::Lines, GameStep::Review]);
    }

    #[test]
    fn step_without_work_is_empty() {
        let work = StepWork { step: GameStep::Lines, left: vec![WorkCount { label: "реплик без перевода", count: 0 }], requests: 0 };
        assert!(work.is_empty());
    }
}
