//! Разбор и проверка ответа модели.

use super::{Batch, Draft, LinesMode, Scene};
use crate::glossary::Term;

/// Ответ модели: номера реплик с текстом и новые термины.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinesAnswer {
    /// Номер реплики в пакете и текст.
    pub lines: Vec<(usize, String)>,
    /// Название и перевод нового термина.
    pub terms: Vec<(String, String)>,
}

impl LinesAnswer {
    /// JSON ответа, а сломанный (комментарии, пояснения вне строк, пропущенные кавычки) запасным разбором
    /// пар номера и перевода (`line_answer`). Перевод принимается и из поля `translation`.
    pub fn parse(text: &str) -> Self {
        todo!()
    }

    /// Отбрасывает перевод без токенов и тегов оригинала (`usable_translation`), правку редактуры без изменений
    /// (`review_fixes`) и длинный перевод, совпавший с переводом реплики с другим оригиналом (`accepted_lines`).
    pub fn accept(&self, mode: LinesMode, scene: &Scene, batch: &Batch) -> Vec<Draft> {
        todo!()
    }

    /// Новые термины, которые есть в тексте пакета: в глоссарий они попадут предположениями (`new_terms`).
    pub fn new_terms(&self, scene: &Scene, batch: &Batch) -> Vec<Term> {
        todo!()
    }
}
