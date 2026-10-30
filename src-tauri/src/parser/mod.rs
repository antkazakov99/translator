use crate::graph::ScenarioGraph;
use anyhow::Result;
use std::path::Path;

pub mod renpy;

/// Универсальный контракт парсера для любого игрового движка.
pub trait GameParser {
    /// Имя парсера (например, "renpy", "rpgmaker", "unity")
    fn name(&self) -> &'static str;

    /// Парсит файл перевода или сценария и строит из него граф ScenarioGraph
    fn parse_file(&self, path: &Path, content: &str) -> Result<ScenarioGraph>;
}
