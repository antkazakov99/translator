# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Translator: настольное приложение на Tauri 2 (Rust) и Svelte 5 для перевода игр на русский, с графом сцен, глоссарием и LLM: локальными (LM Studio, Ollama) и внешними (ChatGPT, Claude, Gemini). Сейчас поддерживается Ren'Py, другие движки подключаются через трейт `GameEngine`.

## Состояние

Идёт переход на 2.0: проект один раз считывает игру и дальше работает со своей копией в SQLite. Код 1.0 в Rust удалён. Классы 2.0 описаны, но команды Tauri ещё не подключены: окно Svelte вызывает команды 1.0, которых больше нет. Тела с `todo!()` ждут реализации. Атрибут `allow(dead_code, unused_variables, unused_imports)` в `lib.rs` снимается вместе с подключением команд.

Проектная документация лежит не в репозитории, а в Docmost: страница «Структура приложения» (слои, классы, сценарии) и раздел «Хранилище 2.0» (схемы app.db, project.db, cache.db). Код следует им. Если решение меняется, сначала обновляется страница.

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
cargo test --lib test_corrupt_segment_sizes_do_not_overflow
cargo test --lib -- --ignored
cargo clippy --lib --tests
```

Тесты с `#[ignore]` в `engines/renpy` работают на реальных играх и берут пути из переменных окружения: `TRANSLATOR_TEST_TRANSLATED_GAME`, `TRANSLATOR_TEST_RPA_GAME`, `TRANSLATOR_TEST_CHARACTERS_GAME`, `TRANSLATOR_TEST_RPA2_ARCHIVE`, `TRANSLATOR_TEST_FRAMED_ARCHIVE`, `TRANSLATOR_TEST_TL_DIR`, `TRANSLATOR_TEST_PACKED_TL_GAME`. Без переменной такой тест ничего не проверяет и проходит. `TRANSLATOR_RPYC_PROBE_DIR` включает запись архивов для проверки скомпилированных скриптов настоящим движком.

## Устройство Rust

Папки по фичам, слои это файлы внутри фичи (`model.rs`, `repo.rs`, `service.rs`), в каждой папке `mod.rs`:

- `projects/`: список проектов, создание, открытие, смена папки игры, удаление.
- `scan/`: снимок игры в project.db, сопоставление реплик после повторного считывания (`matching.rs`), «Пропавшие».
- `llm/`: провайдеры и профили, действия и их настройки, цель запроса (`LlmTarget`), HTTP (`LlmClient`), запуск (`LlmRun`, `RunSlot`).
- `translation/`: перевод и редактура реплик: пакеты, промпт, разбор ответа, черновики и история.
- `glossary/`: говорящие и связи с персонажами, персонажи, термины, токены, описания сцен, заметки модели, `NameKey`.
- `name_rules/`, `fonts/`, `export/`: правила имён, замены шрифтов, сборка и установка мода.
- `engines/`: порт `GameEngine` и типы снимка. `engines/renpy/` это адаптер Ren'Py: `rpa` (архивы RPA-2.0 и 3.0), `rpyc` (декомпиляция), `rpyc_writer` (компиляция `.rpyc` без игры), `pickle`, `script` (разбор `.rpy`) и `graph` перенесены из 1.0. Считывание (`snapshot.rs`) идёт в два шага: `collect_folder` кладёт нужные файлы в `FileStore` как есть (скрипты, `tl/russian`, картинки перевода, шрифты; файл на диске важнее архива, архивы с конца алфавита, `.rpyc` рядом со своим `.rpy` не нужен), `analyze` собирает снимок только из `FileStore`, поэтому пересборка без папки игры идёт тем же путём. Реплики берутся из `tl/russian`, если он есть, иначе из скриптов. Ключ реплики это JSON `LineKey`: `Block` по `id`, `Strings` и `Loose` (вне меток) с путём файла, `Script` по метке. Порядок сцен по сюжету в `scenes.rs`.
- `storage/`: `Database` и обёртки `AppDb`, `ProjectDb`, `CacheDb`. Миграции app.db и project.db (`migrations.rs`) это SQL-файлы `storage/schema/<база>/NNNN_<имя>.sql` в явном списке `APP` и `PROJECT`, история применённых шагов лежит в таблице `schema_migrations`. Новый шаг добавляется в конец, выпущенный шаг не меняется. cache.db без миграций: `schema/cache.sql` и `CACHE_VERSION`. `files.rs` это `FileStore`, `secrets.rs` это `SecretStore`, `state.rs` это `AppState` и `OpenProject`, `common.rs` это общие типы и ошибки.

### Правила слоёв

- **Ядро без ввода-вывода.** Модели держат поля приватными и проверяют правила сами, неверные данные из базы дают `ValidationError`, а не панику. Чистые функции (пакеты, промпты, разбор ответа, сопоставление) тоже в ядре.
- **Репозиторий** это `struct XRepo<'c> { conn: &'c Connection }` на одну операцию: только SQL над своими таблицами, без транзакций и правил. Трейтов у репозиториев нет, тесты идут на настоящей SQLite.
- **Сервис** задаёт порядок шагов, открывает транзакции (`Database::read`, `Database::write`) и делает ввод-вывод. Транзакция короткая: ответ модели и чтение игры идут вне её. Одна транзакция работает с одной базой, `ATTACH` не используется.
- **Ответ модели** пишется только в реплику, которая осталась в том виде, в каком ушла модели (`Draft.sent`).
- **Запуск модели** один за раз во всём приложении: место занимает `LlmRun::begin` через `RunSlot`.
- **Ключи API** знает только `SecretStore`. `ApiKey` без `Debug` и `Serialize`.
- **Файлы в базе не хранятся:** содержимое лежит в `projects/<id>/files/<sha256>`, в базе только сведения о файле.
- **Ren'Py только в `engines/renpy`.** Хранилище и общий код не опираются на раскладку папок движка. Ключи контейнеров, реплик и говорящих база хранит как непрозрачные строки.
- **Совместимости с 1.0 нет:** данные 1.0 не читаются и не переносятся.

## Главные инварианты

- **Панику в Rust нельзя допускать на данных игры и базы.** В релизе `panic = "abort"`, поэтому паника закрывает приложение. Файлы игр недоверенные: разбор возвращает `Err`, архивы проверяются на выход за пределы папки (`safe_output_path`), `unwrap` на значениях из базы запрещён.
- **CSP действует только в сборке.** В `tauri dev` страница грузится с dev-сервера напрямую, поэтому правила из `tauri.conf.json` проверяются на `tauri build`. Интерфейс не обращается к сети сам: запросы к нейросети идут из Rust.
- **Вызовы JS API Tauri**, кроме собственных команд, требуют разрешений в `src-tauri/capabilities/default.json`: `core:default` и `opener:default` покрывают не всё.
- **Если меняется вывод декомпилятора или разбора**, поднимается `PARSER_VERSION` в `engines/renpy/mod.rs`: разбор в cache.db и снимок пересобираются из `files/`. `CACHE_VERSION` в `rpa.rs` остался от дискового кэша 1.0 (`refresh_script_cache`) и уйдёт вместе с ним.

## Соглашения

- Комментарии и сообщения для пользователя пишутся на русском.
- Команды Tauri возвращают `Result<_, String>` (`AppError` превращается в текст), работу с диском и базой выполняют в `spawn_blocking`.
- Сообщения коммитов следуют Conventional Commits на английском.
