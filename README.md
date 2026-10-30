# Translator

Universal localization & translation tool for game engines (Ren'Py, RPG Maker, Unity) with local LLMs (LM Studio, Ollama).
Built with Rust.

The game directory is read-only for Translator. Extracted scripts, the glossary, and
default exports are stored under `%LOCALAPPDATA%\TranslatorApp\projects_cache` on
Windows. Custom export and extraction destinations must be outside the game directory.
Refreshing the script cache does not delete the game's `.rpyc` files.

The exported mod contains the saved edits: edits of `tl/russian` files are applied to copies
packed into the archive, and edits of other scripts become string translations in
`tl/russian/translator_saved_strings.rpy`.
