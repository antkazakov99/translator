use flate2::read::ZlibDecoder;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::Path;

use crate::parser::renpy::RenpyParser;
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
    pub kind: String, // "say", "label", "menu", "jump", "call"
    pub scene_name: String,
    pub speaker: Option<String>,
    pub text: String,
    pub line_number: usize,
    pub target: Option<String>,
}

pub struct RpycParser;

const MAX_RPYC_BYTES: u64 = 128 * 1024 * 1024;
const MAX_AST_BYTES: usize = 256 * 1024 * 1024;

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
            if data[0] == 0x78
                && (data[1] == 0x9c || data[1] == 0x01 || data[1] == 0x5e || data[1] == 0xda)
            {
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
        let end = start
            .checked_add(slot_info.length as usize)
            .ok_or("Смещение слота RPYC переполнено")?;

        if end > data.len() {
            return Err(format!(
                "Смещение слота {}..{} выходит за пределы файла ({})",
                start,
                end,
                data.len()
            ));
        }

        let slot_bytes = &data[start..end];
        let decoder = ZlibDecoder::new(slot_bytes);
        let mut decompressed = Vec::new();
        decoder
            .take(MAX_AST_BYTES as u64 + 1)
            .read_to_end(&mut decompressed)
            .map_err(|e| format!("Ошибка распаковки zlib слота {}: {}", slot_info.slot, e))?;
        if decompressed.len() > MAX_AST_BYTES {
            return Err(format!(
                "Распакованный слот {} RPYC слишком велик",
                slot_info.slot
            ));
        }

        Ok(decompressed)
    }

    /// Декомпилирует/парсит .rpyc файл и извлекает все диалоговые инструкции
    pub fn parse_rpyc_file(path: &Path) -> Result<Vec<RpycStatement>, String> {
        let file = fs::File::open(path)
            .map_err(|e| format!("Не удалось открыть .rpyc {:?}: {}", path, e))?;
        let mut data = Vec::new();
        file.take(MAX_RPYC_BYTES + 1)
            .read_to_end(&mut data)
            .map_err(|e| format!("Не удалось прочитать .rpyc {:?}: {}", path, e))?;
        if data.len() as u64 > MAX_RPYC_BYTES {
            return Err(format!("Файл .rpyc {:?} слишком велик", path));
        }
        Self::parse_rpyc_bytes(&data)
    }

    /// Декомпилирует/парсит байты .rpyc из памяти и извлекает диалоговые инструкции
    pub fn parse_rpyc_bytes(data: &[u8]) -> Result<Vec<RpycStatement>, String> {
        let root_val = Self::load_tree(data)?;

        let mut statements = Vec::new();
        let mut current_scene = "Общие диалоги".to_string();

        Self::extract_statements_from_pickle(&root_val, &mut current_scene, &mut statements);

        Ok(statements)
    }

    fn load_tree(data: &[u8]) -> Result<PickleValue, String> {
        let slots = Self::read_slots(data)?;
        let slot1 = slots
            .iter()
            .find(|s| s.slot == 1)
            .ok_or_else(|| "В файле .rpyc отсутствует Slot 1 (AST байткод)".to_string())?;

        let decompressed_ast = Self::decompress_slot(data, slot1)?;

        let mut decoder = PickleDecoder::new(&decompressed_ast);
        decoder
            .decode()
            .map_err(|e| format!("Ошибка парсинга AST pickle: {}", e))
    }

    /// Собирает реплики оригинала по идентификаторам блоков `translate`.
    pub fn collect_original_dialogue(
        data: &[u8],
        originals: &mut HashMap<String, String>,
    ) -> Result<(), String> {
        let tree = Self::load_tree(data)?;
        let statements = Self::tree_statements(&tree)?;
        DialogueIdentifiers::new(originals).walk(statements);
        Ok(())
    }

    fn tree_statements(tree: &PickleValue) -> Result<&[PickleValue], String> {
        tree.as_tuple()
            .and_then(|root| root.get(1))
            .and_then(PickleValue::as_list)
            .ok_or_else(|| "Неизвестная структура скрипта .rpyc".to_string())
    }

    fn class_of(node: &PickleValue) -> &str {
        match node {
            PickleValue::Object { class_name, .. } => class_name,
            _ => "",
        }
    }

    fn state_of(node: &PickleValue) -> Option<&PickleValue> {
        match node {
            PickleValue::Object { state, .. } => state.as_deref(),
            _ => None,
        }
    }

    fn language_of(state: Option<&PickleValue>) -> Option<&str> {
        Self::get_field_val(state, "language").and_then(PickleValue::as_str)
    }

    fn words(state: Option<&PickleValue>, field: &str) -> Vec<String> {
        match Self::get_field_val(state, field) {
            Some(PickleValue::Tuple(items) | PickleValue::List(items)) => items
                .iter()
                .filter_map(PickleValue::as_str)
                .map(str::to_string)
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Аргументы реплики в виде `("base", xpos="far_left")`.
    fn arguments_code(info: &PickleValue) -> Option<String> {
        let state = Self::state_of(info);
        let Some(PickleValue::Tuple(items) | PickleValue::List(items)) =
            Self::get_field_val(state, "arguments")
        else {
            return None;
        };
        let indexes = |field: &str| match Self::get_field_val(state, field) {
            Some(PickleValue::Set(items)) => {
                items.iter().filter_map(PickleValue::as_i64).collect::<Vec<_>>()
            }
            _ => Vec::new(),
        };
        let (starred, doublestarred) = (indexes("starred_indexes"), indexes("doublestarred_indexes"));

        let mut parts = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let pair = item.as_tuple()?;
            let expression = Self::extract_pycode_source(pair.get(1)?)?;
            parts.push(match pair.first().and_then(PickleValue::as_str) {
                _ if starred.contains(&(index as i64)) => format!("*{}", expression),
                _ if doublestarred.contains(&(index as i64)) => format!("**{}", expression),
                Some(name) => format!("{}={}", name, expression),
                None => expression,
            });
        }
        // Старые движки хранят распаковку в отдельных полях
        for (field, prefix) in [("extrapos", "*"), ("extrakw", "**")] {
            if let Some(expression) =
                Self::get_field_val(state, field).and_then(Self::extract_pycode_source)
            {
                parts.push(format!("{}{}", prefix, expression));
            }
        }
        Some(format!("({})", parts.join(", ")))
    }

    /// Восстанавливает строку реплики как `Say.get_code` в Ren'Py: от этого текста
    /// зависит идентификатор блока, поэтому порядок частей менять нельзя.
    fn say_code(state: Option<&PickleValue>) -> Option<String> {
        let what = Self::get_field_val(state, "what")?.as_str()?;
        let mut parts = Vec::new();
        if let Some(who) = Self::get_field_val(state, "who").and_then(Self::extract_pycode_source) {
            parts.push(who);
            parts.extend(Self::words(state, "attributes"));
            let temporary = Self::words(state, "temporary_attributes");
            if !temporary.is_empty() {
                parts.push("@".to_string());
                parts.extend(temporary);
            }
        }
        parts.push(encode_say_string(what));
        if Self::get_field_val(state, "interact") == Some(&PickleValue::Bool(false)) {
            parts.push("nointeract".to_string());
        }
        if let Some(identifier) = Self::get_field_val(state, "identifier").and_then(PickleValue::as_str) {
            parts.push(format!("id {}", identifier));
        }
        if let Some(info) = Self::get_field_val(state, "arguments")
            && *info != PickleValue::None
        {
            parts.push(Self::arguments_code(info)?);
        }
        if let Some(with) = Self::get_field_val(state, "with_").and_then(Self::extract_pycode_source) {
            parts.push(format!("with {}", with));
        }
        Some(parts.join(" "))
    }

    /// Оператор, который нельзя восстановить, остаётся заметной строкой: файл с ним
    /// не соберётся заново, а молчаливая потеря изменила бы поведение перевода.
    fn opaque_statement(node: &PickleValue) -> String {
        format!("<{}>", Self::class_of(node))
    }

    /// Операторы блока `translate`: признак реплики и текст оператора.
    fn translate_statements(state: Option<&PickleValue>) -> Vec<(bool, String)> {
        let block = Self::get_field_val(state, "block").and_then(PickleValue::as_list);
        block
            .unwrap_or_default()
            .iter()
            .map(|node| {
                let node_state = Self::state_of(node);
                let class = Self::class_of(node);
                let statement = match class {
                    "Say" => Self::say_code(node_state),
                    "Pass" => Some("pass".to_string()),
                    "UserStatement" => Self::get_field_val(node_state, "line")
                        .and_then(PickleValue::as_str)
                        .map(str::to_string),
                    _ => None,
                };
                (class == "Say", statement.unwrap_or_else(|| Self::opaque_statement(node)))
            })
            .collect()
    }

    /// Восстанавливает файл перевода: блоки `translate` с идентификаторами и строки
    /// `old`/`new`. Над репликой стоит комментарий с оригиналом, как в файлах Ren'Py.
    pub fn decompile_translation(
        data: &[u8],
        originals: &HashMap<String, String>,
    ) -> Result<String, String> {
        let tree = Self::load_tree(data)?;
        let statements = Self::tree_statements(&tree)?;

        let mut out = String::new();
        for node in statements {
            let state = Self::state_of(node);
            match Self::class_of(node) {
                "Translate" | "TranslateSay" => {
                    let identifier = Self::get_field_val(state, "identifier").and_then(PickleValue::as_str);
                    let (Some(identifier), Some(language)) = (identifier, Self::language_of(state)) else {
                        out.push_str(&format!("{}\n", Self::opaque_statement(node)));
                        continue;
                    };
                    let statements = if Self::class_of(node) == "TranslateSay" {
                        vec![(true, Self::say_code(state).unwrap_or_else(|| Self::opaque_statement(node)))]
                    } else {
                        Self::translate_statements(state)
                    };
                    out.push_str(&format!("translate {} {}:\n", language, identifier));
                    let mut commented = false;
                    for (is_say, statement) in &statements {
                        if *is_say && !commented {
                            commented = true;
                            let source = originals.get(identifier).unwrap_or(statement);
                            out.push_str(&format!("    # {}\n", source));
                        }
                        out.push_str(&format!("    {}\n", statement));
                    }
                    out.push('\n');
                }
                "EndTranslate" | "Return" => {}
                "Init" => {
                    let block = Self::get_field_val(state, "block").and_then(PickleValue::as_list);
                    let mut language = None;
                    let mut strings = String::new();
                    for item in block.unwrap_or_default() {
                        let item_state = Self::state_of(item);
                        let pair = (
                            Self::get_field_val(item_state, "old").and_then(PickleValue::as_str),
                            Self::get_field_val(item_state, "new").and_then(PickleValue::as_str),
                        );
                        if let ("TranslateString", (Some(old), Some(new))) = (Self::class_of(item), pair) {
                            language = language.or(Self::language_of(item_state));
                            strings.push_str(&format!(
                                "\n    old \"{}\"\n    new \"{}\"\n",
                                RenpyParser::escape_renpy_string(old),
                                RenpyParser::escape_renpy_string(new)
                            ));
                        } else {
                            out.push_str(&format!("{}\n", Self::opaque_statement(item)));
                        }
                    }
                    if let Some(language) = language {
                        out.push_str(&format!("translate {} strings:\n{}\n", language, strings));
                    }
                }
                _ => out.push_str(&format!("{}\n", Self::opaque_statement(node))),
            }
        }
        Ok(out)
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
                        let line_number =
                            Self::get_field_int(state.as_deref(), args, "linenumber").unwrap_or(0);
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
                        let what =
                            Self::get_field_str(state.as_deref(), args, "what").unwrap_or_default();
                        let line_number =
                            Self::get_field_int(state.as_deref(), args, "linenumber").unwrap_or(0);

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
                        let line_number =
                            Self::get_field_int(state.as_deref(), args, "linenumber").unwrap_or(0);
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
                                        Self::extract_statements_from_pickle(
                                            &t[2],
                                            current_scene,
                                            stmts,
                                        );
                                    }
                                }
                            }
                        }
                    }
                    "If" => {
                        let entries_val = Self::get_field_val(state.as_deref(), "entries");
                        if let Some(PickleValue::List(entries)) = entries_val {
                            for e in entries {
                                if let PickleValue::Tuple(t) = e
                                    && t.len() >= 2
                                {
                                    Self::extract_statements_from_pickle(
                                        &t[1],
                                        current_scene,
                                        stmts,
                                    );
                                }
                            }
                        }
                    }
                    "Jump" => {
                        let target = Self::get_field_str(state.as_deref(), args, "target");
                        let line_number =
                            Self::get_field_int(state.as_deref(), args, "linenumber").unwrap_or(0);
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
                        let line_number =
                            Self::get_field_int(state.as_deref(), args, "linenumber").unwrap_or(0);
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
                    "Define" => {
                        let varname = Self::get_field_str(state.as_deref(), args, "varname")
                            .unwrap_or_default();
                        let code_str = Self::get_field_val(state.as_deref(), "code")
                            .and_then(Self::extract_pycode_source)
                            .unwrap_or_default();
                        let line_number =
                            Self::get_field_int(state.as_deref(), args, "linenumber").unwrap_or(0);
                        if !varname.is_empty() {
                            stmts.push(RpycStatement {
                                kind: "define".to_string(),
                                scene_name: varname.clone(),
                                speaker: Some(varname),
                                text: code_str,
                                line_number,
                                target: None,
                            });
                        }
                    }
                    "Default" => {
                        let varname = Self::get_field_str(state.as_deref(), args, "varname")
                            .unwrap_or_default();
                        let code_str = Self::get_field_val(state.as_deref(), "code")
                            .and_then(Self::extract_pycode_source)
                            .unwrap_or_default();
                        let line_number =
                            Self::get_field_int(state.as_deref(), args, "linenumber").unwrap_or(0);
                        if !varname.is_empty() {
                            stmts.push(RpycStatement {
                                kind: "default".to_string(),
                                scene_name: varname.clone(),
                                speaker: None,
                                text: code_str,
                                line_number,
                                target: None,
                            });
                        }
                    }
                    "Python" | "EarlyPython" => {
                        let code_val = Self::get_field_val(state.as_deref(), "code");
                        let py_source = code_val
                            .and_then(Self::extract_pycode_source)
                            .unwrap_or_default();
                        let line_number =
                            Self::get_field_int(state.as_deref(), args, "linenumber").unwrap_or(0);
                        if !py_source.is_empty() {
                            stmts.push(RpycStatement {
                                kind: "python".to_string(),
                                scene_name: current_scene.clone(),
                                speaker: None,
                                text: py_source,
                                line_number,
                                target: None,
                            });
                        }
                    }
                    "Init" => {
                        if let Some(block_val) = Self::get_field_val(state.as_deref(), "block") {
                            Self::extract_statements_from_pickle(block_val, current_scene, stmts);
                        } else if let Some(st) = state {
                            Self::extract_statements_from_pickle(st, current_scene, stmts);
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

    fn get_field_val<'a>(
        state: Option<&'a PickleValue>,
        field_name: &str,
    ) -> Option<&'a PickleValue> {
        match state {
            Some(PickleValue::Dict(entries)) => {
                for (k, v) in entries {
                    if let PickleValue::String(key_str) = k
                        && key_str == field_name
                    {
                        return Some(v);
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
        if let Some(val) = Self::get_field_val(state, field_name)
            && let Some(s) = val.as_str()
        {
            return Some(s.to_string());
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
        if let Some(val) = Self::get_field_val(state, field_name)
            && let Some(i) = val.as_i64()
        {
            return Some(i as usize);
        }
        None
    }

    /// Рекурсивно извлекает исходный код из PyCode или PyExpr объекта pickle
    pub fn extract_pycode_source(val: &PickleValue) -> Option<String> {
        match val {
            PickleValue::String(s) => Some(s.clone()),
            PickleValue::Object {
                class_name,
                state,
                args,
                ..
            } => {
                if class_name == "PyExpr"
                    && let Some(first) = args.first()
                    && let Some(s) = first.as_str()
                {
                    return Some(s.to_string());
                }
                if let Some(st) = state
                    && let Some(s) = Self::extract_pycode_source(st)
                {
                    return Some(s);
                }
                for a in args {
                    if let Some(s) = Self::extract_pycode_source(a) {
                        return Some(s);
                    }
                }
                None
            }
            PickleValue::Tuple(items) | PickleValue::List(items) => {
                if items.len() >= 2
                    && let Some(s) = Self::extract_pycode_source(&items[1])
                {
                    return Some(s);
                }
                for item in items {
                    if let Some(s) = Self::extract_pycode_source(item) {
                        return Some(s);
                    }
                }
                None
            }
            _ => None,
        }
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
                    let escaped_text = RenpyParser::escape_renpy_string(&stmt.text);
                    if let Some(ref spk) = stmt.speaker {
                        out.push_str(&format!("    {} \"{}\"\n", spk, escaped_text));
                    } else {
                        out.push_str(&format!("    \"{}\"\n", escaped_text));
                    }
                }
                "menu" => {
                    let escaped_text = RenpyParser::escape_renpy_string(&stmt.text);
                    out.push_str(&format!(
                        "        \"{}\":\n            pass\n",
                        escaped_text
                    ));
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
                "define" => {
                    out.push_str(&format!("define {} = {}\n", stmt.scene_name, stmt.text));
                }
                "default" => {
                    out.push_str(&format!("default {} = {}\n", stmt.scene_name, stmt.text));
                }
                "python" => {
                    for line in stmt.text.lines() {
                        let trimmed = line.trim();
                        if !trimmed.is_empty() {
                            out.push_str(&format!("$ {}\n", trimmed));
                        }
                    }
                }
                _ => {}
            }
        }

        out
    }
}

/// Запись строки реплики, как `encode_say_string` в Ren'Py.
fn encode_say_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    let mut previous = None;
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '"' => out.push_str("\\\""),
            ' ' if previous == Some(' ') => out.push_str("\\ "),
            _ => out.push(c),
        }
        previous = Some(c);
    }
    out.push('"');
    out
}

/// Вычисляет идентификаторы блоков, как `Restructurer` в Ren'Py: в `.rpyc` игры их нет,
/// движок получает их при загрузке из меток и текста реплик.
struct DialogueIdentifiers<'a> {
    label: Option<String>,
    alternate: Option<String>,
    originals: &'a mut HashMap<String, String>,
}

impl<'a> DialogueIdentifiers<'a> {
    fn new(originals: &'a mut HashMap<String, String>) -> Self {
        Self {
            label: None,
            alternate: None,
            originals,
        }
    }

    fn unique(&self, label: Option<&str>, digest: &str) -> String {
        let base = label.map_or(digest.to_string(), |label| {
            format!("{}_{}", label.replace('.', "_"), digest)
        });
        let mut identifier = base.clone();
        let mut suffix = 0;
        while self.originals.contains_key(&identifier) {
            suffix += 1;
            identifier = format!("{}_{}", base, suffix);
        }
        identifier
    }

    fn finish_group(&mut self, group: &mut Vec<String>, explicit: Option<String>) {
        let Some(code) = group.last().cloned() else {
            return;
        };
        let mut context = md5::Context::new();
        for statement in group.iter() {
            context.consume(statement.as_bytes());
            context.consume(b"\r\n");
        }
        let digest = format!("{:x}", context.compute());
        let digest = &digest[..8];

        let identifier = self.unique(self.label.as_deref(), digest);
        self.originals.insert(identifier, code.clone());
        if let Some(alternate) = self.alternate.clone() {
            let identifier = self.unique(Some(&alternate), digest);
            self.originals.insert(identifier, code.clone());
        }
        if let Some(explicit) = explicit {
            self.originals.insert(explicit, code);
        }
        group.clear();
    }

    fn walk_nested(&mut self, node: &PickleValue) {
        let state = RpycParser::state_of(node);
        match RpycParser::class_of(node) {
            "Label" | "Init" | "While" => {
                if let Some(block) = RpycParser::get_field_val(state, "block").and_then(PickleValue::as_list) {
                    self.walk(block);
                }
            }
            class @ ("If" | "Menu") => {
                let (field, position) = if class == "If" { ("entries", 1) } else { ("items", 2) };
                let entries = RpycParser::get_field_val(state, field).and_then(PickleValue::as_list);
                for entry in entries.unwrap_or_default() {
                    let block = entry.as_tuple().and_then(|entry| entry.get(position));
                    if let Some(block) = block.and_then(PickleValue::as_list) {
                        self.walk(block);
                    }
                }
            }
            _ => {}
        }
    }

    fn walk(&mut self, children: &[PickleValue]) {
        let mut group = Vec::new();
        for node in children {
            let state = RpycParser::state_of(node);
            let class = RpycParser::class_of(node);
            if class == "Label"
                && RpycParser::get_field_val(state, "hide") != Some(&PickleValue::Bool(true))
                && let Some(name) = RpycParser::get_field_val(state, "name")
                    .and_then(PickleValue::as_str)
                    .or_else(|| RpycParser::get_field_val(state, "_name").and_then(PickleValue::as_str))
            {
                if name.starts_with('_') {
                    self.alternate = Some(name.to_string());
                } else {
                    self.label = Some(name.to_string());
                    self.alternate = None;
                }
            }
            self.walk_nested(node);

            let translatable = class == "UserStatement"
                && RpycParser::get_field_val(state, "translatable") == Some(&PickleValue::Bool(true));
            if class == "Say"
                && let Some(code) = RpycParser::say_code(state)
            {
                group.push(code);
                let explicit = RpycParser::get_field_val(state, "identifier")
                    .and_then(PickleValue::as_str)
                    .map(str::to_string);
                self.finish_group(&mut group, explicit);
            } else if translatable {
                let line = RpycParser::get_field_val(state, "line").and_then(PickleValue::as_str);
                group.extend(line.map(str::to_string));
            } else {
                self.finish_group(&mut group, None);
            }
        }
        self.finish_group(&mut group, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::GameParser;

    fn node(class: &str, fields: Vec<(&str, PickleValue)>) -> PickleValue {
        let dict = fields
            .into_iter()
            .map(|(name, value)| (PickleValue::String(name.to_string()), value))
            .collect();
        PickleValue::Object {
            class_module: "renpy.ast".to_string(),
            class_name: class.to_string(),
            state: Some(Box::new(PickleValue::Tuple(vec![PickleValue::None, PickleValue::Dict(dict)]))),
            args: Vec::new(),
        }
    }

    fn say(who: &str, what: &str) -> PickleValue {
        node(
            "Say",
            vec![
                ("who", PickleValue::String(who.to_string())),
                ("what", PickleValue::String(what.to_string())),
                ("interact", PickleValue::Bool(true)),
            ],
        )
    }

    #[test]
    fn computes_identifiers_like_the_engine() {
        // md5("e \"Hello\"\r\n") = 3a1a6e58...
        let digest = format!("{:x}", md5::compute("e \"Hello\"\r\n"));
        let label = node(
            "Label",
            vec![
                ("name", PickleValue::String("start.intro".to_string())),
                ("block", PickleValue::List(vec![say("e", "Hello"), say("e", "Hello")])),
            ],
        );
        let mut originals = HashMap::new();
        DialogueIdentifiers::new(&mut originals).walk(&[label]);

        let first = format!("start_intro_{}", &digest[..8]);
        assert_eq!(originals[&first], "e \"Hello\"");
        assert_eq!(originals[&format!("{first}_1")], "e \"Hello\"");
        assert_eq!(encode_say_string("a  b\n\"c\"\\"), "\"a \\ b\\n\\\"c\\\"\\\\\"");
    }

    #[test]
    fn test_decompiled_text_keeps_line_breaks_and_backslashes() {
        let text = "First \"line\"\nC:\\new";
        let statement = |kind: &str| RpycStatement {
            kind: kind.to_string(),
            scene_name: "start".to_string(),
            speaker: None,
            text: text.to_string(),
            line_number: 1,
            target: None,
        };
        let script = RpycParser::decompile_to_rpy_text(&[statement("say"), statement("menu")]);
        let graph = RenpyParser::new()
            .parse_file(Path::new("story.rpy"), &script)
            .unwrap();
        let texts: Vec<_> = graph.dialogues().map(|line| line.original_text.as_str()).collect();
        assert_eq!(texts, [text, text]);
    }

    #[test]
    #[ignore = "Требуются локальные тестовые файлы игры"]
    fn test_inspect_init_chars_rpyc() {
        let Some(game_dir) = crate::local_test_game(crate::TEST_CHARACTERS_GAME) else {
            return;
        };
        let path = Path::new(&game_dir).join("archive.rpa");

        let archive = crate::rpa::RpaArchive::open(&path).unwrap();

        // Тестируем декомпиляцию define_vars.rpyc с извлечением Define
        if let Some(entry) = archive.entries.get("___sys/_vars/define_vars.rpyc") {
            let mut file = fs::File::open(&path).unwrap();
            let archive_size = file.metadata().unwrap().len();
            let payload = entry.read_compiled(&mut file, archive_size).unwrap();

            let stmts = RpycParser::parse_rpyc_bytes(&payload).unwrap();
            let defines = stmts.iter().filter(|s| s.kind == "define").count();
            println!(
                "Декомпиляция define_vars.rpyc: инструкций {}, из них define: {}",
                stmts.len(),
                defines
            );
            assert!(defines > 0, "Должны быть извлечены Define инструкции!");

            let decompiled = RpycParser::decompile_to_rpy_text(&stmts);
            assert!(decompiled.contains("define lis = Character('Lisa'"));
            assert!(decompiled.contains("define emm = Character('Emma'"));
            println!("Успешно декомпилированы персонажи (Lisa, Emma) в define_vars.rpy!");
        }

        // Тестируем декомпиляцию init_Chars.rpyc с извлечением Python
        if let Some(entry) = archive.entries.get("___sys/_vars/init_Chars.rpyc") {
            let mut file = fs::File::open(&path).unwrap();
            let archive_size = file.metadata().unwrap().len();
            let payload = entry.read_compiled(&mut file, archive_size).unwrap();

            let stmts = RpycParser::parse_rpyc_bytes(&payload).unwrap();
            let py_stmts = stmts.iter().filter(|s| s.kind == "python").count();
            println!(
                "Декомпиляция init_Chars.rpyc: инструкций {}, из них python: {}",
                stmts.len(),
                py_stmts
            );
            assert!(py_stmts > 0, "Должны быть извлечены Python инструкции!");

            let decompiled = RpycParser::decompile_to_rpy_text(&stmts);
            assert!(decompiled.contains("player") && decompiled.contains("Chars('Jack'"));
            println!("Успешно декомпилирован главный герой Jack в init_Chars.rpy!");
        }
    }

    #[test]
    #[ignore = "Требуются локальные тестовые файлы игры"]
    fn test_parse_real_celia_rpyc() {
        let Some(game_dir) = crate::local_test_game(crate::TEST_TRANSLATED_GAME) else {
            return;
        };
        let celia_path = Path::new(&game_dir).join("celia.rpyc");

        let stmts = RpycParser::parse_rpyc_file(&celia_path).expect("Ошибка парсинга celia.rpyc");
        println!("Всего извлечено инструкций из celia.rpyc: {}", stmts.len());
        assert!(
            !stmts.is_empty(),
            "Должны быть извлечены инструкции из celia.rpyc"
        );

        let labels = stmts.iter().filter(|s| s.kind == "label").count();
        let says = stmts.iter().filter(|s| s.kind == "say").count();
        let jumps = stmts.iter().filter(|s| s.kind == "jump").count();
        println!(
            "Метки сцен: {}, Диалоговые реплики: {}, Переходы: {}",
            labels, says, jumps
        );

        for s in stmts.iter().filter(|s| s.kind == "say").take(5) {
            println!(
                "  [Реплика] {}: '{}'",
                s.speaker.as_deref().unwrap_or("Рассказчик"),
                s.text
            );
        }

        assert!(says > 0, "Должны быть извлечены диалоговые реплики");

        let decompiled = RpycParser::decompile_to_rpy_text(&stmts);
        assert!(decompiled.contains("label end_scene_nei:"));
        assert!(decompiled.contains("The neighbor's association now serves a renewed purpose"));
        println!(
            "Успешно декомпилировано в .rpy (символов: {})",
            decompiled.len()
        );
    }
}
