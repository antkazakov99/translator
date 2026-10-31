# Translator

Universal localization & translation tool for game engines (Ren'Py, RPG Maker, Unity) with local LLMs (LM Studio, Ollama).
Built with Rust.

The game directory is read-only for Translator. Extracted scripts, the glossary, and
default exports are stored under `%LOCALAPPDATA%\TranslatorApp\projects_cache` on
Windows. Custom export and extraction destinations must be outside the game directory.
Refreshing the script cache does not delete the game's `.rpyc` files.

The mod is built as `ru_lang.rpa`, which is placed into the game's `game` directory. Ren'Py loads
only compiled `.rpyc` scripts from archives, so the app compiles them itself for the Ren'Py
version of the game, without running the game. Edits of `tl/russian` files are applied to the
compiled copies, and edits of other scripts become string translations. If the game has no
action for this language, the archive also enables it at launch.

The compiler covers string translations, `translate` blocks with plain dialogue and `init python`.
Other statements, such as `voice`, stop the build unless the game already compiled that file.
