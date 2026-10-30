use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use flate2::read::ZlibDecoder;
use serde::{Deserialize, Serialize};

use crate::pickle::{PickleDecoder, PickleValue};

/// Метаданные слота заголовка RPYC
#[derive(Debug, Clone)]
pub struct RpycSlot {
    pub slot: u32,
    pub offset: u32,
    pub length: u32,
}

/// Извлеченная реплика или сцена из скомпилированного .rpyc
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpycStatement {
    pub kind: String,          // "say", "label", "menu", "jump", "call"
    pub scene_name: String,
    pub speaker: Option<String>,
    pub text: String,
    pub line_number: usize,
    pub target: Option<String>,
}

pub struct RpycParser;

impl RpycParser {
    /// Считывает таблицу слотов из .rpyc файла
    pub fn read_slots(data: &[u8]) -> Result<Vec<RpycSlot>, String> {
        if data.len() < 10 {
            return Err("Файл слишком мал для RPYC".to_string());
        }

        // Проверяем заголовок "RENPY RPC2"
        let magic = &data[0..10];
        if magic != b"RENPY RPC2" {
            // Проверяем, не является ли файл сырым zlib потоком (старые версии Ren'Py)
            if data[0] == 0x78 && (data[1] == 0x9c || data[1] == 0x01 || data[1] == 0x5e || data[1] == 0xda) {
                return Ok(vec![RpycSlot {
                    slot: 1,
                    offset: 0,
                    length: data.len() as u32,
                }]);
            }
            return Err("Неверная сигнатура RPYC (ожидалось 'RENPY RPC2')".to_string());
        }

        let mut slots = Vec::new();
        let mut pos = 10;

        while pos + 12 <= data.len() {
            let slot_id = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap());
            let offset = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap());
            let length = u32::from_le_bytes(data[pos + 8..pos + 12].try_into().unwrap());
            pos += 12;

            if slot_id == 0 {
                break;
            }

            slots.push(RpycSlot {
                slot: slot_id,
                offset,
                length,
            });
        }

        Ok(slots)
    }

    /// Распаковывает данные Slot 1 (AST байткод) из .rpyc
    pub fn decompress_slot(data: &[u8], slot_info: &RpycSlot) -> Result<Vec<u8>, String> {
        let start = slot_info.offset as usize;
        let end = start + slot_info.length as usize;

        if end > data.len() {
            return Err(format!(
                "Смещение слота {}..{} выходит за пределы файла ({})",
                start, end, data.len()
            ));
        }

        let slot_bytes = &data[start..end];
        let mut decoder = ZlibDecoder::new(slot_bytes);
        let mut decompressed = Vec::new();
        decoder
            .read_to_end(&mut decompressed)
            .map_err(|e| format!("Ошибка распаковки zlib слота {}: {}", slot_info.slot, e))?;

        Ok(decompressed)
    }

    /// Декомпилирует/парсит .rpyc файл и извлекает все диалоговые инструкции
    pub fn parse_rpyc_file(path: &Path) -> Result<Vec<RpycStatement>, String> {
        let data = fs::read(path)
            .map_err(|e| format!("Не удалось прочитать .rpyc {:?}: {}", path, e))?;
        Self::parse_rpyc_bytes(&data)
    }

    /// Декомпилирует/парсит байты .rpyc из памяти и извлекает диалоговые инструкции
    pub fn parse_rpyc_bytes(data: &[u8]) -> Result<Vec<RpycStatement>, String> {
        let slots = Self::read_slots(data)?;
        let slot1 = slots
            .iter()
            .find(|s| s.slot == 1)
            .ok_or_else(|| "В файле .rpyc отсутствует Slot 1 (AST байткод)".to_string())?;

        let decompressed_ast = Self::decompress_slot(data, slot1)?;

        let mut decoder = PickleDecoder::new(&decompressed_ast);
        let root_val = decoder
            .decode()
            .map_err(|e| format!("Ошибка парсинга AST pickle: {}", e))?;

        let mut statements = Vec::new();
        let mut current_scene = "Общие диалоги".to_string();

        Self::extract_statements_from_pickle(&root_val, &mut current_scene, &mut statements);

        Ok(statements)
    }

    /// Рекурсивно извлекает AST узлы (Say, Label, Menu, Jump, Call) из дерева десериализации Pickle
    fn extract_statements_from_pickle(
        val: &PickleValue,
        current_scene: &mut String,
        stmts: &mut Vec<RpycStatement>,
    ) {
        match val {
            PickleValue::List(items) | PickleValue::Tuple(items) => {
                for item in items {
                    Self::extract_statements_from_pickle(item, current_scene, stmts);
                }
            }
            PickleValue::Object {
                class_name,
                state,
                args,
                ..
            } => {
                match class_name.as_str() {
                    "Label" => {
                        let name = Self::get_field_str(state.as_deref(), args, "name")
                            .or_else(|| Self::get_field_str(state.as_deref(), args, "_name"))
                            .unwrap_or_else(|| "unnamed_scene".to_string());
                        let line_number = Self::get_field_int(state.as_deref(), args, "linenumber").unwrap_or(0);
                        *current_scene = name.clone();

                        stmts.push(RpycStatement {
                            kind: "label".to_string(),
                            scene_name: name.clone(),
                            speaker: None,
                            text: format!("label {}:", name),
                            line_number,
                            target: Some(name),
                        });

                        // Рекурсивно извлекаем тело сцены
                        if let Some(block_val) = Self::get_field_val(state.as_deref(), "block") {
                            Self::extract_statements_from_pickle(block_val, current_scene, stmts);
                        }
                    }
                    "Say" => {
                        let who = Self::get_field_str(state.as_deref(), args, "who");
                        let what = Self::get_field_str(state.as_deref(), args, "what")
                            .unwrap_or_default();
                        let line_number = Self::get_field_int(state.as_deref(), args, "linenumber").unwrap_or(0);

                        stmts.push(RpycStatement {
                            kind: "say".to_string(),
                            scene_name: current_scene.clone(),
                            speaker: who,
                            text: what,
                            line_number,
                            target: None,
                        });
                    }
                    "Menu" => {
                        let line_number = Self::get_field_int(state.as_deref(), args, "linenumber").unwrap_or(0);
                        // Извлекаем элементы меню
                        let items_val = Self::get_field_val(state.as_deref(), "items");
                        if let Some(PickleValue::List(menu_items)) = items_val {
                            for m in menu_items {
                                if let PickleValue::Tuple(t) = m {
                                    if let Some(choice_text) = t.first().and_then(|v| v.as_str()) {
                                        stmts.push(RpycStatement {
                                            kind: "menu".to_string(),
                                            scene_name: current_scene.clone(),
                                            speaker: Some("Выбор".to_string()),
                                            text: choice_text.to_string(),
                                            line_number,
                                            target: None,
                                        });
                                    }
                                    // Если у ветки выбора есть свой блок инструкций, парсим его
                                    if t.len() >= 3 {
                                        Self::extract_statements_from_pickle(&t[2], current_scene, stmts);
                                    }
                                }
                            }
                        }
                    }
                    "If" => {
                        let entries_val = Self::get_field_val(state.as_deref(), "entries");
                        if let Some(PickleValue::List(entries)) = entries_val {
                            for e in entries {
                                if let PickleValue::Tuple(t) = e {
                                    if t.len() >= 2 {
                                        Self::extract_statements_from_pickle(&t[1], current_scene, stmts);
                                    }
                                }
                            }
                        }
                    }
                    "Jump" => {
                        let target = Self::get_field_str(state.as_deref(), args, "target");
                        let line_number = Self::get_field_int(state.as_deref(), args, "linenumber").unwrap_or(0);
                        if let Some(tgt) = target {
                            stmts.push(RpycStatement {
                                kind: "jump".to_string(),
                                scene_name: current_scene.clone(),
                                speaker: None,
                                text: format!("jump {}", tgt),
                                line_number,
                                target: Some(tgt),
                            });
                        }
                    }
                    "Call" => {
                        let target = Self::get_field_str(state.as_deref(), args, "label");
                        let line_number = Self::get_field_int(state.as_deref(), args, "linenumber").unwrap_or(0);
                        if let Some(tgt) = target {
                            stmts.push(RpycStatement {
                                kind: "call".to_string(),
                                scene_name: current_scene.clone(),
                                speaker: None,
                                text: format!("call {}", tgt),
                                line_number,
                                target: Some(tgt),
                            });
                        }
                    }
                    "Translate" => {
                        // Рекурсивно парсим внутренний блок перевода
                        if let Some(block_val) = Self::get_field_val(state.as_deref(), "block") {
                            Self::extract_statements_from_pickle(block_val, current_scene, stmts);
                        }
                    }
                    _ => {
                        // Для других узлов рекурсивно проверяем состояние
                        if let Some(st) = state {
                            Self::extract_statements_from_pickle(st, current_scene, stmts);
                        }
                    }
                }
            }
            PickleValue::Dict(entries) => {
                for (_, v) in entries {
                    Self::extract_statements_from_pickle(v, current_scene, stmts);
                }
            }
            _ => {}
        }
    }

    fn get_field_val<'a>(state: Option<&'a PickleValue>, field_name: &str) -> Option<&'a PickleValue> {
        match state {
            Some(PickleValue::Dict(entries)) => {
                for (k, v) in entries {
                    if let PickleValue::String(key_str) = k {
                        if key_str == field_name {
                            return Some(v);
                        }
                    }
                }
                None
            }
            Some(PickleValue::Tuple(items)) => {
                for item in items {
                    if let Some(val) = Self::get_field_val(Some(item), field_name) {
                        return Some(val);
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn get_field_str(
        state: Option<&PickleValue>,
        args: &[PickleValue],
        field_name: &str,
    ) -> Option<String> {
        if let Some(val) = Self::get_field_val(state, field_name) {
            if let Some(s) = val.as_str() {
                return Some(s.to_string());
            }
        }
        // Запасной вариант: первый строковый аргумент в args
        for arg in args {
            if let Some(s) = arg.as_str() {
                return Some(s.to_string());
            }
        }
        None
    }

    fn get_field_int(
        state: Option<&PickleValue>,
        _args: &[PickleValue],
        field_name: &str,
    ) -> Option<usize> {
        if let Some(val) = Self::get_field_val(state, field_name) {
            if let Some(i) = val.as_i64() {
                return Some(i as usize);
            }
        }
        None
    }

    /// Декомпилирует список инструкций в валидный текстовый скрипт .rpy
    pub fn decompile_to_rpy_text(statements: &[RpycStatement]) -> String {
        let mut out = String::new();
        out.push_str("# Decompiled by Ren'Py Scenario & Localization Studio\n\n");

        for stmt in statements {
            match stmt.kind.as_str() {
                "label" => {
                    out.push_str(&format!("\nlabel {}:\n", stmt.scene_name));
                }
                "say" => {
                    let escaped_text = stmt.text.replace('"', "\\\"");
                    if let Some(ref spk) = stmt.speaker {
                        out.push_str(&format!("    {} \"{}\"\n", spk, escaped_text));
                    } else {
                        out.push_str(&format!("    \"{}\"\n", escaped_text));
                    }
                }
                "menu" => {
                    let escaped_text = stmt.text.replace('"', "\\\"");
                    out.push_str(&format!("        \"{}\":\n            pass\n", escaped_text));
                }
                "jump" => {
                    if let Some(ref target) = stmt.target {
                        out.push_str(&format!("    jump {}\n", target));
                    }
                }
                "call" => {
                    if let Some(ref target) = stmt.target {
                        out.push_str(&format!("    call {}\n", target));
                    }
                }
                _ => {}
            }
        }

        out
    }

    /// Очищает устаревший кэш скомпилированных файлов .rpyc
    /// Если tl_only = true, очищает только папку tl/russian (безопасный режим),
    /// иначе очищает всю директорию игры.
    pub fn clean_cache(dir: &Path, tl_only: bool) -> Result<usize, String> {
        let target_dir = if tl_only {
            dir.join(r"tl\russian")
        } else {
            dir.to_path_buf()
        };

        if !target_dir.exists() {
            return Ok(0);
        }

        let mut rpyc_files = Vec::new();
        collect_rpyc_files(&target_dir, &mut rpyc_files);

        let count = rpyc_files.len();
        for file in rpyc_files {
            let _ = fs::remove_file(&file);
        }

        Ok(count)
    }
}

fn collect_rpyc_files(dir: &Path, files: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_rpyc_files(&path, files);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rpyc") {
                files.push(path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_real_blank_rpyc() {
        let blank_path = Path::new(r"F:\Playnite\Games\0b588a07-1844-467f-8459-e5629642a1bf\Installed\game\blank.rpyc");
        if !blank_path.exists() {
            return;
        }

        let stmts = RpycParser::parse_rpyc_file(blank_path).expect("Ошибка парсинга blank.rpyc");
        let decompiled = RpycParser::decompile_to_rpy_text(&stmts);
        assert!(!decompiled.is_empty());
        println!("Decompiled blank.rpyc:\n{}", decompiled);
    }

    #[test]
    fn test_parse_real_celia_rpyc() {
        let celia_path = Path::new(r"F:\Playnite\Games\0b588a07-1844-467f-8459-e5629642a1bf\Installed\game\celia.rpyc");
        if !celia_path.exists() {
            return;
        }

        let stmts = RpycParser::parse_rpyc_file(celia_path).expect("Ошибка парсинга celia.rpyc");
        println!("Всего извлечено инструкций из celia.rpyc: {}", stmts.len());
        assert!(!stmts.is_empty(), "Должны быть извлечены инструкции из celia.rpyc");

        let labels = stmts.iter().filter(|s| s.kind == "label").count();
        let says = stmts.iter().filter(|s| s.kind == "say").count();
        let jumps = stmts.iter().filter(|s| s.kind == "jump").count();
        println!("Метки сцен: {}, Диалоговые реплики: {}, Переходы: {}", labels, says, jumps);

        for s in stmts.iter().filter(|s| s.kind == "say").take(5) {
            println!("  [Реплика] {}: '{}'", s.speaker.as_deref().unwrap_or("Рассказчик"), s.text);
        }

        assert!(says > 0, "Должны быть извлечены диалоговые реплики");

        let decompiled = RpycParser::decompile_to_rpy_text(&stmts);
        assert!(decompiled.contains("label end_scene_nei:"));
        assert!(decompiled.contains("The neighbor's association now serves a renewed purpose"));
        println!("Успешно декомпилировано в .rpy (символов: {})", decompiled.len());
    }
}
