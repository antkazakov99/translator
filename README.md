# Translator

Universal localization & translation tool for game engines (Ren'Py, RPG Maker, Unity) with local LLMs (LM Studio, Ollama).
Built with Rust.

Name translation in the character glossary has separate buttons: analyze probable origins
and load or generate language rules, then translate and check spellings. A single model selector
can be switched between these stages; translation reuses the completed analysis.
The panel shows probable languages and proposed translations for selection. Rules are edited
in the separate Rules tab. Saved rules are shared between
projects in `%LOCALAPPDATA%\TranslatorApp\name_rules.json`. Ambiguous or unchecked spellings
require manual selection before applying. Manual chatbot requests include saved rules and
glossary spellings, but do not receive the separate API check.
The glossary's Rules tab shows the whole language library, with search, manual editing and
model-generated drafts. Its edits are shared with name translation and saved by Save glossary.
Analysis automatically drafts missing rules as new languages are found. Translation and checking
run one name at a time, including only its probable languages' rules and relevant glossary entries.
Each character card shows the analyzed origins of its given name, surname and other forms.
These hypotheses are saved with the glossary and hidden if the original name or owner context changes.
Bulk analysis only fills missing origins and rules. A character card has separate name analysis
and translation buttons; its analysis can explicitly refresh that character's origins.

The game directory is read-only for Translator. Extracted scripts, the glossary, and
default exports are stored under `%LOCALAPPDATA%\TranslatorApp\projects_cache` on
Windows. Custom export and extraction destinations must be outside the game directory.
Refreshing the script cache does not delete the game's `.rpyc` files. The stored data depends
on the game's path, so after moving a game or unpacking a new version, copy the edits and the
glossary from the old project with the transfer button in the project manager.

The mod is built as `ru_lang.rpa`, which is placed into the game's `game` directory. Ren'Py loads
only compiled `.rpyc` scripts from archives, so the app compiles them itself for the Ren'Py
version of the game, without running the game. Edits of `tl/russian` files are applied to the
compiled copies, and edits of other scripts become string translations. If the game has no
action for this language, the archive also enables it at launch.

A translation packed into the game's `.rpa` archives can be the starting point too: the app
restores its `translate` blocks, matches them to the original lines, and the exported mod carries
the whole translation with its images and fonts, so it works in a copy of the game without it.

Ren'Py reads files on disk before archives, so a `tl/russian` folder on disk wins over the mod.
To use the mod in such a game, remove that folder first.

The compiler covers string translations, `translate` blocks with dialogue, its arguments,
temporary `@` attributes, `nointeract` and `with` clauses, and `init python`.
Other statements, such as `voice` or `*` unpacking in arguments, stop the build unless the game
already compiled that file.
