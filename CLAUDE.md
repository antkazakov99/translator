# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Translator: настольное приложение на Tauri 2 (Rust) и Svelte 5 для перевода игр на Ren'Py на русский, с графом сцен, глоссарием и локальными LLM (LM Studio, Ollama). Парсер спрятан за трейтом `GameParser`, но реализован только Ren'Py.

## Команды

Из корня проекта:

```bash
npm install
npm run tauri dev
npm run tauri build
npm run check
node --test tests/*.test.mjs
node --test --test-name-pattern="focused mode" tests/scene-graph.test.mjs
```

Скрипта `npm test` нет. Тесты импортируют `.ts` напрямую, поэтому нужен Node с нативной загрузкой TypeScript.

Из `src-tauri`:

```bash
cargo test --lib
cargo test --lib test_saved_edits_survive_line_shifts
cargo test --lib -- --ignored
cargo clippy
```

Тесты с `#[ignore]` работают на реальных играх и берут пути из переменных окружения: `TRANSLATOR_TEST_TRANSLATED_GAME`, `TRANSLATOR_TEST_RPA_GAME`, `TRANSLATOR_TEST_CHARACTERS_GAME`, `TRANSLATOR_TEST_RPA2_ARCHIVE`, `TRANSLATOR_TEST_FRAMED_ARCHIVE`, `TRANSLATOR_TEST_TL_DIR`. Без переменной такой тест ничего не проверяет и проходит. `TRANSLATOR_RPYC_PROBE_DIR` включает запись архивов для проверки скомпилированных скриптов настоящим движком.

## Архитектура

Интерфейс почти целиком находится в `src/routes/+page.svelte`. Это SPA без SSR. Команды Tauri объявлены в `src-tauri/src/lib.rs` и регистрируются в `generate_handler!`. Вызовы JS API Tauri, кроме собственных команд, требуют разрешений в `src-tauri/capabilities/default.json`: `core:default` и `opener:default` покрывают не всё.

### Главные инварианты

- **Папка игры только для чтения.** Всё пишется в `%LOCALAPPDATA%\TranslatorApp\projects_cache\<имя>_<хэш пути>`: `scripts/` (скрипты из RPA), `translations/*.json` (правки), `glossary.json`, `exports/`, `unpacked/`. Вывод в игру запрещает `ensure_output_outside_game`.
- **Панику в Rust нельзя допускать на данных игры.** В релизе `panic = "abort"`, поэтому паника закрывает приложение. Файлы игр считаются недоверенными: разбор возвращает `Err`, а архивы проверяются на выход за пределы папки (`safe_output_path`).
- **Язык задан константой.** `TL_LANG` в `lib.rs` равен `russian`, то же значение записано в `SourceViewerModal.svelte`.

### Поток данных

1. **Скан** (`scan_game_project`): скрипты из `.rpa` распаковываются в `scripts/`, `.rpyc` декомпилируются в `.rpy`, собираются персонажи и список файлов. Приоритет источников: `tl/russian`, затем кэш, затем файлы `.rpy` в папке игры. Свой архив `ru_lang.rpa` при этом пропускается.
2. **Разбор** (`parse_project_file`): `RenpyParser` строит `ScenarioGraph` из файла перевода или оригинала, а структура сцен и переходов берётся из оригинала (`resolve_original_script`, в том числе из кэша). Результат кэшируется по времени изменения файлов.
3. **Правки** (`translation.rs`): игровые файлы не меняются, правки лежат в JSON по ключу `LineKey`. Он не зависит от номера строки: `Block` для блоков `translate`, `Strings` для `old`, `Script` для оригинальных скриптов. Старые ключи-массивы переносятся при загрузке.
4. **Экспорт** (`build_mod_archive`):
   - Правки файлов `tl/<язык>` подставляются в копии этих файлов.
   - Остальные правки становятся строковыми переводами в `translator_saved_strings.rpy`, потому что `id` блоков `translate` вычисляет сам Ren'Py.
   - `translator_language.rpy` включает язык, если в игре нет такого пункта.
   - Всё компилируется и упаковывается в `ru_lang.rpa`, потому что из архивов Ren'Py читает только `.rpyc`.

### Форматы Ren'Py

- `pickle.rs`: декодер и кодировщик Python pickle для индексов RPA и деревьев скриптов.
- `rpa.rs`: чтение и запись архивов RPA-2.0 и RPA-3.0.
- `rpyc.rs`: декомпиляция в упрощённый `.rpy` только для разбора, не для повторной компиляции игрой. Если вывод декомпилятора меняется, нужно поднять `CACHE_VERSION` в `rpa.rs`, иначе останется старый кэш.
- `rpyc_writer.rs`: компилятор `.rpyc` без запуска игры. Формат определяется по файлам движка (`ScriptFormat::detect`), поддерживаются три раскладки `AstLayout`. Компилируются только строковые переводы, блоки `translate` с репликами (включая `nointeract` и `with`) и `init python`, остальное останавливает сборку.

### Состояние в памяти

Статические кэши в `lib.rs` (`PARSED_FILE_CACHE`, `SCRIPT_STRUCTURE_CACHE`, `PROJECT_CACHE`) сбрасываются при сохранении правок и очистке кэша. `find_game_root` определяет корень игры по `cached_script_projects` и эвристикам, а от него зависит путь к хранилищу правок. Запись одного файла или папки сериализуется через `path_lock`.

## Соглашения

- Комментарии и сообщения для пользователя пишутся на русском.
- Команды Tauri возвращают `Result<_, String>`, а работу с диском выполняют в `spawn_blocking`.
- Сообщения коммитов следуют Conventional Commits на английском.
