use std::path::{Path, PathBuf};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use anyhow::{Context, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EngineType {
    Renpy,
    RpgMaker,
    Unity,
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    pub api_url: String,
    pub model: String,
    pub temperature: f32,
    pub max_tokens: u32,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            api_url: "http://localhost:1234/v1".to_string(),
            model: "mistral-small".to_string(),
            temperature: 0.3,
            max_tokens: 2000,
        }
    }
}

/// Манифест проекта локализации.
/// Описывает игру, целевой язык, путь к оригиналу и настройки перевода.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectManifest {
    pub id: String,
    pub name: String,
    pub engine: EngineType,
    /// Путь к установленной оригинальной игре (Read-Only источник)
    pub game_dir: PathBuf,
    pub target_lang: String,
    pub version: String,
    pub llm: LlmConfig,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ProjectManifest {
    pub fn new(id: impl Into<String>, name: impl Into<String>, engine: EngineType, game_dir: PathBuf, target_lang: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: id.into(),
            name: name.into(),
            engine,
            game_dir,
            target_lang: target_lang.into(),
            version: "0.1.0".to_string(),
            llm: LlmConfig::default(),
            created_at: now,
            updated_at: now,
        }
    }

    /// Папка проекта (где лежат манифест, данные и собранный мод)
    pub fn project_dir(&self, base_projects_dir: &Path) -> PathBuf {
        base_projects_dir.join(&self.id)
    }

    /// Папка с внутренними данными проекта (граф, глоссарий, база кеша)
    pub fn data_dir(&self, base_projects_dir: &Path) -> PathBuf {
        self.project_dir(base_projects_dir).join("data")
    }

    /// Папка со сборкой мода (Staging Area перед установкой в игру)
    pub fn dist_dir(&self, base_projects_dir: &Path) -> PathBuf {
        self.project_dir(base_projects_dir).join("dist")
    }

    /// Путь к файлу project.json
    pub fn manifest_path(&self, base_projects_dir: &Path) -> PathBuf {
        self.project_dir(base_projects_dir).join("project.json")
    }

    /// Сохранить манифест на диск
    pub fn save(&self, base_projects_dir: &Path) -> Result<()> {
        let path = self.manifest_path(base_projects_dir);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Не удалось создать директорию проекта: {:?}", parent))?;
        }
        let json = serde_json::to_string_pretty(self)
            .context("Не удалось сериализовать манифест проекта")?;
        std::fs::write(&path, json)
            .with_context(|| format!("Не удалось записать манифест: {:?}", path))?;
        Ok(())
    }

    /// Загрузить манифест с диска
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Не удалось прочитать манифест: {:?}", path))?;
        let manifest = serde_json::from_str(&content)
            .with_context(|| format!("Не удалось распарсить JSON манифеста: {:?}", path))?;
        Ok(manifest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_serialization() {
        let manifest = ProjectManifest::new(
            "take-over",
            "Take Over",
            EngineType::Renpy,
            PathBuf::from("F:/Games/Take Over"),
            "russian",
        );

        let json = serde_json::to_string_pretty(&manifest).unwrap();
        assert!(json.contains("\"engine\": \"renpy\""));
        assert!(json.contains("\"target_lang\": \"russian\""));

        let loaded: ProjectManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.id, "take-over");
        assert_eq!(loaded.engine, EngineType::Renpy);
    }
}
