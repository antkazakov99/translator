//! Команды замены шрифтов.

use std::path::PathBuf;

use serde::Serialize;
use tauri::AppHandle;

use super::{FontCoverage, FontService, GameFont};
use crate::common::AppError;
use crate::files::ContentHash;
use crate::projects::ProjectId;
use crate::state::blocking;

/// Каких символов нет в шрифте.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FontCoverageDto {
    /// Шрифт не прочитан: о символах ничего не известно.
    pub unreadable: bool,
    pub missing_original: String,
    pub missing_target: String,
}

impl From<FontCoverage> for FontCoverageDto {
    fn from(coverage: FontCoverage) -> Self {
        Self { unreadable: coverage.unreadable, missing_original: coverage.missing_original, missing_target: coverage.missing_target }
    }
}

/// Замена шрифта.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FontReplacementDto {
    /// Имя загруженного файла.
    pub source_name: String,
    pub hash: String,
    pub coverage: FontCoverageDto,
}

/// Шрифт игры.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct GameFontDto {
    /// Путь файла снимка: по нему ставится и убирается замена.
    pub path: String,
    /// Путь для показа, без папки `game/`.
    pub name: String,
    pub hash: String,
    /// Сколько скриптов упоминают файл.
    pub references: u32,
    pub coverage: FontCoverageDto,
    /// Почему замена не подействует.
    pub locked: Option<String>,
    pub replacement: Option<FontReplacementDto>,
}

impl From<GameFont> for GameFontDto {
    fn from(font: GameFont) -> Self {
        Self {
            name: font.path.strip_prefix("game/").unwrap_or(&font.path).to_string(),
            path: font.path,
            hash: font.hash.as_str().to_string(),
            references: u32::try_from(font.mentions).unwrap_or(u32::MAX),
            coverage: font.coverage.into(),
            locked: font.locked,
            replacement: font.replacement.map(|(replacement, coverage)| FontReplacementDto {
                source_name: replacement.file_name,
                hash: replacement.hash.as_str().to_string(),
                coverage: coverage.into(),
            }),
        }
    }
}

fn project(id: &str) -> Result<ProjectId, AppError> {
    Ok(ProjectId::parse(id)?)
}

/// Шрифты игры с недостающими символами и заменами.
#[tauri::command]
#[specta::specta]
pub async fn game_fonts(app: AppHandle, project: String) -> Result<Vec<GameFontDto>, String> {
    blocking(app, move |state| Ok(FontService { state }.list(self::project(&project)?)?.into_iter().map(GameFontDto::from).collect())).await
}

/// Копирует выбранный файл в проект и ставит его заменой шрифта игры.
#[tauri::command]
#[specta::specta]
pub async fn replace_font(app: AppHandle, project: String, target: String, file: String) -> Result<(), String> {
    blocking(app, move |state| FontService { state }.replace(self::project(&project)?, &target, &PathBuf::from(file)).map(|_| ())).await
}

#[tauri::command]
#[specta::specta]
pub async fn remove_font_replacement(app: AppHandle, project: String, target: String) -> Result<(), String> {
    blocking(app, move |state| FontService { state }.remove(self::project(&project)?, &target)).await
}

/// Файл шрифта для предпросмотра в base64.
#[tauri::command]
#[specta::specta]
pub async fn font_data(app: AppHandle, project: String, hash: String) -> Result<String, String> {
    blocking(app, move |state| {
        let hash = ContentHash::parse(&hash)?;
        Ok(base64(&FontService { state }.data(self::project(&project)?, &hash)?))
    })
    .await
}

/// Base64 со стандартным алфавитом и дополнением.
fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let bytes = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let value = u32::from(bytes[0]) << 16 | u32::from(bytes[1]) << 8 | u32::from(bytes[2]);
        for index in 0..4 {
            if index <= chunk.len() {
                out.push(ALPHABET[(value >> (18 - 6 * index) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::base64;

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xff, 0xfe, 0x00]), "//4A");
    }
}
