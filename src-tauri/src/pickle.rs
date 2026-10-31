use std::collections::{HashMap, HashSet};

/// Представление десериализованного объекта Python Pickle
#[derive(Debug, Clone, PartialEq)]
pub enum PickleValue {
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Bytes(Vec<u8>),
    List(Vec<PickleValue>),
    Tuple(Vec<PickleValue>),
    Dict(Vec<(PickleValue, PickleValue)>),
    Set(Vec<PickleValue>),
    Global {
        module: String,
        name: String,
    },
    Object {
        class_module: String,
        class_name: String,
        state: Option<Box<PickleValue>>,
        args: Vec<PickleValue>,
    },
}

impl PickleValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            PickleValue::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            PickleValue::Int(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            PickleValue::Int(i) if *i >= 0 => Some(*i as u64),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            PickleValue::Bytes(b) => Some(b.as_slice()),
            PickleValue::String(s) => Some(s.as_bytes()),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[PickleValue]> {
        match self {
            PickleValue::List(l) => Some(l.as_slice()),
            _ => None,
        }
    }

    pub fn as_tuple(&self) -> Option<&[PickleValue]> {
        match self {
            PickleValue::Tuple(t) => Some(t.as_slice()),
            _ => None,
        }
    }

    pub fn as_dict(&self) -> Option<&[(PickleValue, PickleValue)]> {
        match self {
            PickleValue::Dict(d) => Some(d.as_slice()),
            _ => None,
        }
    }

    pub fn get_dict_value(&self, key: &str) -> Option<&PickleValue> {
        if let PickleValue::Dict(entries) = self {
            for (k, v) in entries {
                if let PickleValue::String(k_str) = k
                    && k_str == key
                {
                    return Some(v);
                }
            }
        }
        None
    }
}

/// Стек машины состояний Pickle
enum StackItem {
    Mark,
    Val(Slot),
}

struct Slot {
    value: PickleValue,
    depth: usize,
    /// Индексы memo изменяемого объекта, на который поток ссылается позже: его заполняют
    /// уже после записи в memo, поэтому копия обновляется, когда он уходит со стека.
    memo_ids: Vec<usize>,
}

impl StackItem {
    fn leaf(value: PickleValue) -> Self {
        Self::built(value, 1)
    }

    fn built(value: PickleValue, depth: usize) -> Self {
        StackItem::Val(Slot { value, depth, memo_ids: Vec::new() })
    }
}

/// Глубина контейнера с содержимым глубины `inner`. Предел проверяется при разборе:
/// слишком глубокое значение переполнит стек уже при освобождении памяти, а такое
/// падение нельзя перехватить. Рекурсивные обходы результата тоже опираются на этот предел.
fn nested_depth(inner: usize) -> Result<usize, String> {
    let depth = inner + 1;
    if depth > MAX_PICKLE_DEPTH {
        return Err(format!("Слишком глубокая вложенность данных pickle: больше {}", MAX_PICKLE_DEPTH));
    }
    Ok(depth)
}

/// Изменяет объект на вершине стека, добавляя в него содержимое глубины `inner`.
fn update_top(
    stack: &mut [StackItem],
    inner: usize,
    update: impl FnOnce(&mut PickleValue),
) -> Result<(), String> {
    if let Some(StackItem::Val(slot)) = stack.last_mut() {
        slot.depth = slot.depth.max(nested_depth(inner)?);
        update(&mut slot.value);
    }
    Ok(())
}

/// Сколько узлов могут занять копии из memo: иначе несколько килобайт вложенных
/// ссылок разворачиваются в миллиарды объектов и приложение падает без памяти.
const MAX_MEMO_NODES: usize = 5_000_000;

/// Самая глубокая из проверенных игр даёт 107.
pub const MAX_PICKLE_DEPTH: usize = 256;

fn node_count(value: &PickleValue) -> usize {
    1 + match value {
        PickleValue::List(items) | PickleValue::Tuple(items) | PickleValue::Set(items) => {
            items.iter().map(node_count).sum()
        }
        PickleValue::Dict(entries) => {
            entries.iter().map(|(key, value)| node_count(key) + node_count(value)).sum()
        }
        PickleValue::Object { state, args, .. } => {
            state.as_deref().map_or(0, node_count) + args.iter().map(node_count).sum::<usize>()
        }
        _ => 0,
    }
}

/// Декодер Python Pickle (поддерживает протоколы 0, 1, 2, 3, 4, 5)
pub struct PickleDecoder<'a> {
    data: &'a [u8],
    pos: usize,
    memo: HashMap<usize, (PickleValue, usize)>,
    /// Индексы memo, которые поток читает: только такие объекты нужно обновлять.
    referenced: HashSet<usize>,
    /// Индексы объектов, которые ещё на стеке.
    on_stack: HashSet<usize>,
    memo_nodes: usize,
}

impl<'a> PickleDecoder<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            memo: HashMap::new(),
            referenced: HashSet::new(),
            on_stack: HashSet::new(),
            memo_nodes: 0,
        }
    }

    /// Проходит поток без построения объектов и запоминает индексы чтений из memo.
    /// Ошибку не возвращает: тот же сбой покажет основной разбор.
    fn collect_memo_reads(&mut self) -> Result<(), String> {
        loop {
            match self.read_byte()? {
                b'.' => return Ok(()),
                b'h' => {
                    let idx = self.read_byte()? as usize;
                    self.referenced.insert(idx);
                }
                b'j' => {
                    let idx = self.read_u32_le()? as usize;
                    self.referenced.insert(idx);
                }
                0x80 | b'K' | b'q' => {
                    self.read_byte()?;
                }
                b'M' => {
                    self.read_bytes(2)?;
                }
                b'J' | b'r' => {
                    self.read_bytes(4)?;
                }
                b'G' | 0x95 => {
                    self.read_bytes(8)?;
                }
                b'I' | b'F' | b'S' => {
                    self.read_line()?;
                }
                b'c' => {
                    self.read_line()?;
                    self.read_line()?;
                }
                b'U' | b'C' | 0x8a | 0x8c => {
                    let len = self.read_byte()? as usize;
                    self.read_bytes(len)?;
                }
                b'T' | b'X' | b'B' | 0x8b => {
                    let len = self.read_u32_le()? as usize;
                    self.read_bytes(len)?;
                }
                0x8d => {
                    let len = self.read_u64_le()? as usize;
                    self.read_bytes(len)?;
                }
                _ => {}
            }
        }
    }

    pub fn decode(&mut self) -> Result<PickleValue, String> {
        let mut reader = PickleDecoder::new(self.data);
        reader.pos = self.pos;
        let _ = reader.collect_memo_reads();
        self.referenced = reader.referenced;

        let mut stack: Vec<StackItem> = Vec::new();

        while self.pos < self.data.len() {
            let opcode = self.read_byte()?;

            match opcode {
                // PROTO (протокол)
                0x80 => {
                    let _version = self.read_byte()?;
                }

                // STOP
                b'.' => {
                    return match stack.pop().and_then(|item| self.settle(item)) {
                        Some((val, _)) => Ok(val),
                        None => Err("Стек пуст при выполнении STOP".to_string()),
                    };
                }

                // MARK
                b'(' => {
                    stack.push(StackItem::Mark);
                }

                // POP
                b'0' => {
                    if let Some(item) = stack.pop() {
                        self.settle(item);
                    }
                }

                // POP_MARK
                b'1' => {
                    self.pop_to_mark(&mut stack);
                }

                // DUP
                b'2' => {
                    if let Some(StackItem::Val(slot)) = stack.last() {
                        let clone = StackItem::built(slot.value.clone(), slot.depth);
                        stack.push(clone);
                    }
                }

                // NONE
                b'N' => {
                    stack.push(StackItem::leaf(PickleValue::None));
                }

                // NEWTRUE / NEWFALSE / BOOL
                0x88 => stack.push(StackItem::leaf(PickleValue::Bool(true))),
                0x89 => stack.push(StackItem::leaf(PickleValue::Bool(false))),

                // INT (ASCII decimal)
                b'I' => {
                    let line = self.read_line()?;
                    let trimmed = line.trim();
                    if trimmed == "01" {
                        stack.push(StackItem::leaf(PickleValue::Bool(true)));
                    } else if trimmed == "00" {
                        stack.push(StackItem::leaf(PickleValue::Bool(false)));
                    } else {
                        let val: i64 = trimmed.parse().unwrap_or(0);
                        stack.push(StackItem::leaf(PickleValue::Int(val)));
                    }
                }

                // BININT (4-byte i32 LE)
                b'J' => {
                    let val = self.read_i32_le()? as i64;
                    stack.push(StackItem::leaf(PickleValue::Int(val)));
                }

                // BININT1 (1-byte u8)
                b'K' => {
                    let val = self.read_byte()? as i64;
                    stack.push(StackItem::leaf(PickleValue::Int(val)));
                }

                // BININT2 (2-byte u16 LE)
                b'M' => {
                    let val = self.read_u16_le()? as i64;
                    stack.push(StackItem::leaf(PickleValue::Int(val)));
                }

                // LONG1 (1-byte length followed by bytes)
                0x8a => {
                    let len = self.read_byte()? as usize;
                    let bytes = self.read_bytes(len)?;
                    let val = parse_two_complement_i64(bytes);
                    stack.push(StackItem::leaf(PickleValue::Int(val)));
                }

                // LONG4 (4-byte length followed by bytes)
                0x8b => {
                    let len = self.read_u32_le()? as usize;
                    let bytes = self.read_bytes(len)?;
                    let val = parse_two_complement_i64(bytes);
                    stack.push(StackItem::leaf(PickleValue::Int(val)));
                }

                // FLOAT (ASCII decimal)
                b'F' => {
                    let line = self.read_line()?;
                    let val: f64 = line.trim().parse().unwrap_or(0.0);
                    stack.push(StackItem::leaf(PickleValue::Float(val)));
                }

                // BINFLOAT (8-byte f64 BE)
                b'G' => {
                    let bytes = self.read_bytes(8)?;
                    let val = f64::from_be_bytes(bytes.try_into().unwrap());
                    stack.push(StackItem::leaf(PickleValue::Float(val)));
                }

                // STRING (ASCII string in quotes)
                b'S' => {
                    let line = self.read_line()?;
                    let s = unescape_python_str(&line);
                    stack.push(StackItem::leaf(PickleValue::String(s)));
                }

                // BINSTRING (4-byte len + bytes)
                b'T' => {
                    let len = self.read_u32_le()? as usize;
                    let bytes = self.read_bytes(len)?;
                    let s = String::from_utf8_lossy(bytes).into_owned();
                    stack.push(StackItem::leaf(PickleValue::String(s)));
                }

                // SHORT_BINSTRING (1-byte len + bytes)
                b'U' => {
                    let len = self.read_byte()? as usize;
                    let bytes = self.read_bytes(len)?;
                    let s = String::from_utf8_lossy(bytes).into_owned();
                    stack.push(StackItem::leaf(PickleValue::String(s)));
                }

                // BINUNICODE (4-byte len + utf8 bytes)
                b'X' => {
                    let len = self.read_u32_le()? as usize;
                    let bytes = self.read_bytes(len)?;
                    let s = String::from_utf8_lossy(bytes).into_owned();
                    stack.push(StackItem::leaf(PickleValue::String(s)));
                }

                // SHORT_BINUNICODE (1-byte len + utf8 bytes, Protocol 4)
                0x8c => {
                    let len = self.read_byte()? as usize;
                    let bytes = self.read_bytes(len)?;
                    let s = String::from_utf8_lossy(bytes).into_owned();
                    stack.push(StackItem::leaf(PickleValue::String(s)));
                }

                // BINUNICODE8 (8-byte len + utf8 bytes, Protocol 4)
                0x8d => {
                    let len = self.read_u64_le()? as usize;
                    let bytes = self.read_bytes(len)?;
                    let s = String::from_utf8_lossy(bytes).into_owned();
                    stack.push(StackItem::leaf(PickleValue::String(s)));
                }

                // BINBYTES (4-byte len + bytes)
                b'B' => {
                    let len = self.read_u32_le()? as usize;
                    let bytes = self.read_bytes(len)?.to_vec();
                    stack.push(StackItem::leaf(PickleValue::Bytes(bytes)));
                }

                // SHORT_BINBYTES (1-byte len + bytes)
                b'C' => {
                    let len = self.read_byte()? as usize;
                    let bytes = self.read_bytes(len)?.to_vec();
                    stack.push(StackItem::leaf(PickleValue::Bytes(bytes)));
                }

                // EMPTY_TUPLE
                b')' => {
                    stack.push(StackItem::leaf(PickleValue::Tuple(Vec::new())));
                }

                // TUPLE (поглощает всё до MARK)
                b't' => {
                    let (items, depth) = self.pop_to_mark(&mut stack);
                    stack.push(StackItem::built(PickleValue::Tuple(items), nested_depth(depth)?));
                }

                // TUPLE1
                0x85 => {
                    let (v, depth) = self.pop_val(&mut stack)?;
                    stack.push(StackItem::built(PickleValue::Tuple(vec![v]), nested_depth(depth)?));
                }

                // TUPLE2
                0x86 => {
                    let (v2, depth2) = self.pop_val(&mut stack)?;
                    let (v1, depth1) = self.pop_val(&mut stack)?;
                    let depth = nested_depth(depth1.max(depth2))?;
                    stack.push(StackItem::built(PickleValue::Tuple(vec![v1, v2]), depth));
                }

                // TUPLE3
                0x87 => {
                    let (v3, depth3) = self.pop_val(&mut stack)?;
                    let (v2, depth2) = self.pop_val(&mut stack)?;
                    let (v1, depth1) = self.pop_val(&mut stack)?;
                    let depth = nested_depth(depth1.max(depth2).max(depth3))?;
                    stack.push(StackItem::built(PickleValue::Tuple(vec![v1, v2, v3]), depth));
                }

                // EMPTY_LIST
                b']' => {
                    stack.push(StackItem::leaf(PickleValue::List(Vec::new())));
                }

                // LIST (поглощает всё до MARK)
                b'l' => {
                    let (items, depth) = self.pop_to_mark(&mut stack);
                    stack.push(StackItem::built(PickleValue::List(items), nested_depth(depth)?));
                }

                // APPEND
                b'a' => {
                    let (item, depth) = self.pop_val(&mut stack)?;
                    update_top(&mut stack, depth, |top| {
                        if let PickleValue::List(list) = top {
                            list.push(item);
                        }
                    })?;
                }

                // APPENDS (добавляет все элементы до MARK в список)
                b'e' => {
                    let (items, depth) = self.pop_to_mark(&mut stack);
                    update_top(&mut stack, depth, |top| {
                        if let PickleValue::List(list) = top {
                            list.extend(items);
                        }
                    })?;
                }

                // EMPTY_DICT
                b'}' => {
                    stack.push(StackItem::leaf(PickleValue::Dict(Vec::new())));
                }

                // DICT (поглощает пары ключ-значение до MARK)
                b'd' => {
                    let (items, depth) = self.pop_to_mark(&mut stack);
                    let mut dict = Vec::new();
                    let mut iter = items.into_iter();
                    while let Some(k) = iter.next() {
                        if let Some(v) = iter.next() {
                            dict.push((k, v));
                        }
                    }
                    stack.push(StackItem::built(PickleValue::Dict(dict), nested_depth(depth)?));
                }

                // SETITEM (dict[k] = v)
                b's' => {
                    let (val, val_depth) = self.pop_val(&mut stack)?;
                    let (key, key_depth) = self.pop_val(&mut stack)?;
                    update_top(&mut stack, val_depth.max(key_depth), |top| {
                        if let PickleValue::Dict(dict) = top {
                            dict.push((key, val));
                        }
                    })?;
                }

                // SETITEMS (поглощает пары ключ-значение до MARK и кладёт в dict)
                b'u' => {
                    let (items, depth) = self.pop_to_mark(&mut stack);
                    update_top(&mut stack, depth, |top| {
                        if let PickleValue::Dict(dict) = top {
                            let mut iter = items.into_iter();
                            while let Some(k) = iter.next() {
                                if let Some(v) = iter.next() {
                                    dict.push((k, v));
                                }
                            }
                        }
                    })?;
                }

                // EMPTY_SET
                0x8f => {
                    stack.push(StackItem::leaf(PickleValue::Set(Vec::new())));
                }

                // ADDITEMS
                0x90 => {
                    let (items, depth) = self.pop_to_mark(&mut stack);
                    update_top(&mut stack, depth, |top| {
                        if let PickleValue::Set(set) = top {
                            set.extend(items);
                        }
                    })?;
                }

                // BINPUT (1-byte memo index)
                b'q' => {
                    let idx = self.read_byte()? as usize;
                    self.memo_put(&mut stack, idx);
                }

                // LONG_BINPUT (4-byte memo index)
                b'r' => {
                    let idx = self.read_u32_le()? as usize;
                    self.memo_put(&mut stack, idx);
                }

                // MEMOIZE (Protocol 4 auto memo index)
                0x94 => {
                    let idx = self.memo.len();
                    self.memo_put(&mut stack, idx);
                }

                // BINGET (1-byte memo get)
                b'h' => {
                    let idx = self.read_byte()? as usize;
                    let (value, depth) = self.memo_get(&stack, idx)?;
                    stack.push(StackItem::built(value, depth));
                }

                // LONG_BINGET (4-byte memo get)
                b'j' => {
                    let idx = self.read_u32_le()? as usize;
                    let (value, depth) = self.memo_get(&stack, idx)?;
                    stack.push(StackItem::built(value, depth));
                }

                // GLOBAL (module\n name\n)
                b'c' => {
                    let module = self.read_line()?.trim().to_string();
                    let name = self.read_line()?.trim().to_string();
                    stack.push(StackItem::leaf(PickleValue::Global { module, name }));
                }

                // STACK_GLOBAL (Protocol 4: pops name, pops module)
                0x93 => {
                    let name = self
                        .pop_val(&mut stack)?
                        .0
                        .as_str()
                        .unwrap_or_default()
                        .to_string();
                    let module = self
                        .pop_val(&mut stack)?
                        .0
                        .as_str()
                        .unwrap_or_default()
                        .to_string();
                    stack.push(StackItem::leaf(PickleValue::Global { module, name }));
                }

                // REDUCE (callable, args_tuple)
                b'R' => {
                    let (args_val, args_depth) = self.pop_val(&mut stack)?;
                    let (callable, _) = self.pop_val(&mut stack)?;
                    let depth = nested_depth(args_depth)?;
                    let args = match args_val {
                        PickleValue::Tuple(t) => t,
                        PickleValue::List(l) => l,
                        other => vec![other],
                    };

                    match callable {
                        PickleValue::Global { module, name } => {
                            let object = PickleValue::Object {
                                class_module: module,
                                class_name: name,
                                state: None,
                                args,
                            };
                            stack.push(StackItem::built(object, depth));
                        }
                        other => {
                            let object = PickleValue::Object {
                                class_module: "unknown".to_string(),
                                class_name: format!("{:?}", other),
                                state: None,
                                args,
                            };
                            stack.push(StackItem::built(object, depth));
                        }
                    }
                }

                // BUILD (inst, state)
                b'b' => {
                    let (state, depth) = self.pop_val(&mut stack)?;
                    update_top(&mut stack, depth, |top| {
                        if let PickleValue::Object { state: obj_state, .. } = top {
                            *obj_state = Some(Box::new(state));
                        }
                    })?;
                }

                // NEWOBJ (Protocol 2: cls, args)
                0x81 => {
                    let (args_val, args_depth) = self.pop_val(&mut stack)?;
                    let args = match args_val {
                        PickleValue::Tuple(t) => t,
                        other => vec![other],
                    };
                    let depth = nested_depth(args_depth)?;
                    let (cls, _) = self.pop_val(&mut stack)?;
                    let (module, name) = match cls {
                        PickleValue::Global { module, name } => (module, name),
                        _ => ("unknown".to_string(), "unknown".to_string()),
                    };
                    let object = PickleValue::Object {
                        class_module: module,
                        class_name: name,
                        state: None,
                        args,
                    };
                    stack.push(StackItem::built(object, depth));
                }

                // FRAME (Protocol 4): длина блока, данные идут сразу за ней
                0x95 => {
                    self.read_u64_le()?;
                }

                // Пропуск опкода сбил бы разбор: его аргументы читались бы как команды
                other => {
                    return Err(format!(
                        "Неподдерживаемый опкод pickle 0x{:02x} на позиции {}",
                        other,
                        self.pos - 1
                    ));
                }
            }
        }

        // Если файл закончился без опкода STOP, возвращаем верхний элемент стека
        match stack.pop().and_then(|item| self.settle(item)) {
            Some((val, _)) => Ok(val),
            None => Err("Пустой поток pickle данных".to_string()),
        }
    }

    fn memo_put(&mut self, stack: &mut [StackItem], idx: usize) {
        let Some(StackItem::Val(slot)) = stack.last_mut() else {
            return;
        };
        self.memo.insert(idx, (slot.value.clone(), slot.depth));
        let is_mutable = matches!(
            slot.value,
            PickleValue::List(_) | PickleValue::Dict(_) | PickleValue::Set(_) | PickleValue::Object { .. }
        );
        if is_mutable && self.referenced.contains(&idx) {
            self.on_stack.insert(idx);
            slot.memo_ids.push(idx);
        }
    }

    /// Копия объекта из memo. Незавершённый объект ещё на стеке, там он полнее, чем в memo.
    fn memo_get(&mut self, stack: &[StackItem], idx: usize) -> Result<(PickleValue, usize), String> {
        let open = self.on_stack.contains(&idx).then(|| {
            stack.iter().rev().find_map(|item| match item {
                StackItem::Val(slot) if slot.memo_ids.contains(&idx) => Some((&slot.value, slot.depth)),
                _ => None,
            })
        });
        let found = open.flatten().or_else(|| self.memo.get(&idx).map(|(value, depth)| (value, *depth)));
        let Some((value, depth)) = found else {
            return Ok((PickleValue::None, 1));
        };
        self.memo_nodes += node_count(value);
        if self.memo_nodes > MAX_MEMO_NODES {
            return Err("Слишком много повторяющихся объектов в данных pickle".to_string());
        }
        Ok((value.clone(), depth))
    }

    /// Снимает значение со стека и обновляет его копии в memo.
    fn settle(&mut self, item: StackItem) -> Option<(PickleValue, usize)> {
        let StackItem::Val(slot) = item else {
            return None;
        };
        for id in slot.memo_ids {
            self.on_stack.remove(&id);
            self.memo.insert(id, (slot.value.clone(), slot.depth));
        }
        Some((slot.value, slot.depth))
    }

    fn pop_val(&mut self, stack: &mut Vec<StackItem>) -> Result<(PickleValue, usize), String> {
        while let Some(item) = stack.pop() {
            if let Some(value) = self.settle(item) {
                return Ok(value);
            }
        }
        Err("Ожидалось значение в стеке pickle".to_string())
    }

    /// Значения до отметки и наибольшая глубина среди них.
    fn pop_to_mark(&mut self, stack: &mut Vec<StackItem>) -> (Vec<PickleValue>, usize) {
        let mut items = Vec::new();
        let mut depth = 0;
        while let Some(item) = stack.pop() {
            match self.settle(item) {
                None => break,
                Some((value, item_depth)) => {
                    items.push(value);
                    depth = depth.max(item_depth);
                }
            }
        }
        items.reverse();
        (items, depth)
    }

    fn read_byte(&mut self) -> Result<u8, String> {
        if self.pos >= self.data.len() {
            return Err("Неожиданный конец потока данных".to_string());
        }
        let b = self.data[self.pos];
        self.pos += 1;
        Ok(b)
    }

    fn read_bytes(&mut self, len: usize) -> Result<&'a [u8], String> {
        let end = self
            .pos
            .checked_add(len)
            .filter(|end| *end <= self.data.len())
            .ok_or("Неожиданный конец потока данных при чтении среза байт")?;
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn read_line(&mut self) -> Result<String, String> {
        let start = self.pos;
        while self.pos < self.data.len() && self.data[self.pos] != b'\n' {
            self.pos += 1;
        }
        let line = String::from_utf8_lossy(&self.data[start..self.pos]).into_owned();
        if self.pos < self.data.len() && self.data[self.pos] == b'\n' {
            self.pos += 1;
        }
        Ok(line)
    }

    fn read_u16_le(&mut self) -> Result<u16, String> {
        let bytes = self.read_bytes(2)?;
        Ok(u16::from_le_bytes(bytes.try_into().unwrap()))
    }

    fn read_i32_le(&mut self) -> Result<i32, String> {
        let bytes = self.read_bytes(4)?;
        Ok(i32::from_le_bytes(bytes.try_into().unwrap()))
    }

    fn read_u32_le(&mut self) -> Result<u32, String> {
        let bytes = self.read_bytes(4)?;
        Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
    }

    fn read_u64_le(&mut self) -> Result<u64, String> {
        let bytes = self.read_bytes(8)?;
        Ok(u64::from_le_bytes(bytes.try_into().unwrap()))
    }
}

/// Парсинг дополнительного кода (two's complement) для опкодов LONG1 / LONG4
fn parse_two_complement_i64(bytes: &[u8]) -> i64 {
    if bytes.is_empty() {
        return 0;
    }
    let is_negative = (bytes[bytes.len() - 1] & 0x80) != 0;
    let mut val: i64 = 0;
    for (i, &b) in bytes.iter().take(8).enumerate() {
        val |= (b as i64) << (i * 8);
    }
    if is_negative && bytes.len() < 8 {
        // Знаковое расширение
        val |= (-1i64) << (bytes.len() * 8);
    }
    val
}

/// Распаковка экранированных строк Python Pickle (S opcode)
fn unescape_python_str(s: &str) -> String {
    let trimmed = s.trim();
    let quoted = (trimmed.starts_with('\'') && trimmed.ends_with('\''))
        || (trimmed.starts_with('"') && trimmed.ends_with('"'));
    let inner = if quoted && trimmed.len() >= 2 {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    };

    inner
        .replace("\\n", "\n")
        .replace("\\r", "\r")
        .replace("\\t", "\t")
        .replace("\\\\", "\\")
        .replace("\\'", "'")
        .replace("\\\"", "\"")
}

/// Сериализатор Python Pickle Protocol 2 для генерации валидных RPA-3.0 архивов
pub struct PickleEncoder {
    buf: Vec<u8>,
}

impl PickleEncoder {
    pub fn new() -> Self {
        // Заголовок Protocol 2
        Self {
            buf: vec![0x80, 0x02],
        }
    }

    /// Сериализует словарь RPA-3.0 индекса: { filename: [ (offset, length) ] }.
    /// Третьего элемента `prefix` нет: его тип `bytes` появился в протоколе 3,
    /// и unpickler Python 2 в Ren'Py 6/7 не смог бы прочитать индекс.
    pub fn encode_rpa_index(&mut self, entries: &[(&str, u64, u64)]) -> Vec<u8> {
        // EMPTY_DICT
        self.buf.push(b'}');
        // BINPUT 0
        self.buf.push(b'q');
        self.buf.push(0);

        for (name, offset, length) in entries {
            // Ключ: BINUNICODE
            self.write_binunicode(name);

            // Значение: List с одним Tuple2
            self.buf.push(b']'); // EMPTY_LIST
            self.buf.push(b'('); // MARK

            // offset
            self.write_int(*offset as i64);
            // length
            self.write_int(*length as i64);

            self.buf.push(0x86); // TUPLE2
            self.buf.push(b'e'); // APPENDS to List
            self.buf.push(b's'); // SETITEM in Dict
        }

        // STOP
        self.buf.push(b'.');
        self.buf.clone()
    }

    /// Сериализует значение целиком. Объекты без `args` создаются через NEWOBJ и BUILD,
    /// как их записывает Python, остальные вызываются через REDUCE.
    pub fn encode_value(&mut self, value: &PickleValue) -> Vec<u8> {
        self.write_value(value);
        self.buf.push(b'.');
        self.buf.clone()
    }

    fn write_value(&mut self, value: &PickleValue) {
        match value {
            PickleValue::None => self.buf.push(b'N'),
            PickleValue::Bool(true) => self.buf.push(0x88),
            PickleValue::Bool(false) => self.buf.push(0x89),
            PickleValue::Int(val) => self.write_int(*val),
            PickleValue::String(s) => self.write_binunicode(s),
            PickleValue::Tuple(items) => self.write_tuple(items),
            PickleValue::List(items) => {
                self.buf.push(b']');
                if !items.is_empty() {
                    self.buf.push(b'(');
                    for item in items {
                        self.write_value(item);
                    }
                    self.buf.push(b'e');
                }
            }
            PickleValue::Dict(entries) => {
                self.buf.push(b'}');
                if !entries.is_empty() {
                    self.buf.push(b'(');
                    for (key, val) in entries {
                        self.write_value(key);
                        self.write_value(val);
                    }
                    self.buf.push(b'u');
                }
            }
            PickleValue::Global { module, name } => self.write_global(module, name),
            PickleValue::Object {
                class_module,
                class_name,
                state,
                args,
            } => {
                self.write_global(class_module, class_name);
                self.write_tuple(args);
                self.buf.push(if args.is_empty() { 0x81 } else { b'R' });
                if let Some(state) = state {
                    self.write_value(state);
                    self.buf.push(b'b');
                }
            }
            PickleValue::Float(_) | PickleValue::Bytes(_) | PickleValue::Set(_) => {
                unreachable!("значение не встречается в скомпилированных скриптах")
            }
        }
    }

    fn write_tuple(&mut self, items: &[PickleValue]) {
        match items {
            [] => self.buf.push(b')'),
            [_] | [_, _] | [_, _, _] => {
                for item in items {
                    self.write_value(item);
                }
                self.buf.push(0x84 + items.len() as u8);
            }
            _ => {
                self.buf.push(b'(');
                for item in items {
                    self.write_value(item);
                }
                self.buf.push(b't');
            }
        }
    }

    fn write_global(&mut self, module: &str, name: &str) {
        self.buf.push(b'c');
        self.buf.extend_from_slice(module.as_bytes());
        self.buf.push(b'\n');
        self.buf.extend_from_slice(name.as_bytes());
        self.buf.push(b'\n');
    }

    fn write_binunicode(&mut self, s: &str) {
        let bytes = s.as_bytes();
        self.buf.push(b'X');
        self.buf
            .extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        self.buf.extend_from_slice(bytes);
    }

    fn write_int(&mut self, val: i64) {
        if (0..=255).contains(&val) {
            self.buf.push(b'K');
            self.buf.push(val as u8);
        } else if (256..=65535).contains(&val) {
            self.buf.push(b'M');
            self.buf.extend_from_slice(&(val as u16).to_le_bytes());
        } else if (i32::MIN as i64..=i32::MAX as i64).contains(&val) {
            self.buf.push(b'J');
            self.buf.extend_from_slice(&(val as i32).to_le_bytes());
        } else {
            // LONG1 (8 bytes)
            self.buf.push(0x8a);
            self.buf.push(8);
            self.buf.extend_from_slice(&val.to_le_bytes());
        }
    }
}

impl Default for PickleEncoder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_PICKLE_DEPTH, PickleDecoder, PickleValue};

    #[test]
    fn test_shared_list_keeps_items_added_after_memo() {
        let (one, two) = (PickleValue::Int(1), PickleValue::Int(2));
        let filled = PickleValue::List(vec![one.clone(), two.clone()]);

        // Ссылка, пока список ещё на стеке: как при записи пары `(items, items)`
        let data = [0x80, 2, b'(', b']', b'q', 0, b'(', b'K', 1, b'K', 2, b'e', b'h', 0, b't', b'.'];
        assert_eq!(
            PickleDecoder::new(&data).decode().unwrap(),
            PickleValue::Tuple(vec![filled.clone(), filled.clone()])
        );

        // Ссылка после того, как список ушёл со стека внутрь другого объекта
        let data = [b'(', b']', b'q', 0, b'(', b'K', 1, b'K', 2, b'e', 0x85, b'h', 0, b't', b'.'];
        assert_eq!(
            PickleDecoder::new(&data).decode().unwrap(),
            PickleValue::Tuple(vec![PickleValue::Tuple(vec![filled.clone()]), filled])
        );
    }

    #[test]
    fn test_shared_object_keeps_state_built_after_memo() {
        let mut data = b"(cmod\nCls\n)\x81q\x00}b".to_vec();
        data.extend_from_slice(&[0x85, b'h', 0, b't', b'.']);

        let PickleValue::Tuple(items) = PickleDecoder::new(&data).decode().unwrap() else {
            panic!("ожидался кортеж");
        };
        let PickleValue::Object { state, .. } = &items[1] else {
            panic!("ожидался объект");
        };
        assert!(state.is_some());
    }

    #[test]
    fn test_deep_nesting_is_rejected_without_stack_overflow() {
        let mut wrapped = vec![b'K', 1];
        wrapped.extend(std::iter::repeat_n(0x85, 2_000_000));
        wrapped.push(b'.');

        let mut listed = vec![b']'];
        for _ in 0..100_000 {
            listed.extend(*b"]a");
        }
        listed.push(b'.');

        // Глубина растёт через memo: каждый кортеж содержит предыдущий
        let mut rebuilt = b"K\x01q\x00".to_vec();
        for _ in 0..100_000 {
            rebuilt.extend(b"(h\x00tq\x00");
        }
        rebuilt.push(b'.');

        for data in [wrapped, rebuilt] {
            let error = PickleDecoder::new(&data).decode().unwrap_err();
            assert!(error.contains("вложенность"), "{error}");
        }
        // Плоский список соседних списков не вложен и остаётся допустимым
        assert!(PickleDecoder::new(&listed).decode().is_ok());
    }

    #[test]
    fn test_deepest_allowed_value_survives_every_walk() {
        let mut data = vec![b'K', 1];
        data.extend(std::iter::repeat_n(0x85, MAX_PICKLE_DEPTH - 1));
        data.push(b'.');

        let run = move || {
            let value = PickleDecoder::new(&data).decode().unwrap();
            assert_eq!(value.clone(), value);
            crate::rpyc::RpycParser::statements_of(&value).unwrap();
        };
        // Команды Tauri и rayon работают на потоках с небольшим стеком, память освобождается там же
        std::thread::Builder::new().stack_size(1024 * 1024).spawn(run).unwrap().join().unwrap();
    }

    #[test]
    fn test_nested_references_cannot_exhaust_memory() {
        let mut data = vec![b'K', 1, b'q', 0];
        for _ in 0..30 {
            data.push(b'(');
            data.extend([b'h', 0].repeat(8));
            data.extend([b't', b'q', 0]);
        }
        data.push(b'.');

        let error = PickleDecoder::new(&data).decode().unwrap_err();
        assert!(error.contains("повторяющихся"), "{error}");
    }

    use super::*;

    #[test]
    fn frame_lengths_are_not_read_as_opcodes() {
        let frame = |body: &[u8]| {
            let mut bytes = vec![0x95];
            bytes.extend_from_slice(&(body.len() as u64).to_le_bytes());
            bytes.extend_from_slice(body);
            bytes
        };
        // Длина среднего блока равна опкоду STOP
        let data = [
            vec![0x80, 5],
            frame(b"]("),
            frame(&b"K\x01".repeat(usize::from(b'.') / 2)),
            frame(b"e."),
        ]
        .concat();
        assert_eq!(
            PickleDecoder::new(&data).decode().unwrap(),
            PickleValue::List(vec![PickleValue::Int(1); usize::from(b'.') / 2])
        );
    }

    #[test]
    fn unknown_opcode_is_an_error() {
        let error = PickleDecoder::new(&[0x80, 2, 0xff, b'.']).decode().unwrap_err();
        assert!(error.contains("0xff"), "{error}");
    }

    #[test]
    fn corrupt_data_returns_errors_instead_of_panicking() {
        let mut huge_length = vec![0x8d];
        huge_length.extend_from_slice(&u64::MAX.to_le_bytes());
        assert!(PickleDecoder::new(&huge_length).decode().is_err());

        assert_eq!(
            PickleDecoder::new(b"S'\n.").decode().unwrap(),
            PickleValue::String("'".to_string())
        );
    }

    #[test]
    fn encoded_values_decode_back() {
        let object = PickleValue::Object {
            class_module: "renpy.ast".into(),
            class_name: "Say".into(),
            state: Some(Box::new(PickleValue::Tuple(vec![
                PickleValue::None,
                PickleValue::Dict(vec![(
                    PickleValue::String("what".into()),
                    PickleValue::String("Привет".into()),
                )]),
            ]))),
            args: Vec::new(),
        };
        let root = PickleValue::Tuple(vec![
            PickleValue::List(vec![object, PickleValue::List(Vec::new())]),
            PickleValue::Bool(true),
            PickleValue::Int(70_000),
            PickleValue::Tuple(vec![PickleValue::None; 4]),
        ]);
        let bytes = PickleEncoder::new().encode_value(&root);
        assert_eq!(PickleDecoder::new(&bytes).decode().unwrap(), root);

        let reduced = PickleValue::Object {
            class_module: "collections".into(),
            class_name: "defaultdict".into(),
            state: None,
            args: vec![PickleValue::Global {
                module: "builtins".into(),
                name: "list".into(),
            }],
        };
        let bytes = PickleEncoder::new().encode_value(&reduced);
        assert_eq!(PickleDecoder::new(&bytes).decode().unwrap(), reduced);
    }

    #[test]
    fn rpa_index_uses_only_protocol_2_opcodes() {
        let bytes = PickleEncoder::new().encode_rpa_index(&[("a.rpy", 34, 300)]);
        let expected: &[u8] = &[
            0x80, 2, b'}', b'q', 0, b'X', 5, 0, 0, 0, b'a', b'.', b'r', b'p', b'y', b']', b'(',
            b'K', 34, b'M', 0x2c, 0x01, 0x86, b'e', b's', b'.',
        ];
        assert_eq!(bytes, expected);

        let index = PickleDecoder::new(&bytes).decode().unwrap();
        let segments = index.get_dict_value("a.rpy").unwrap().as_list().unwrap();
        assert_eq!(
            segments[0],
            PickleValue::Tuple(vec![PickleValue::Int(34), PickleValue::Int(300)])
        );
    }
}
