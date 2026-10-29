use std::path::Path;
use std::time::Instant;
use translator::parser::renpy::RenpyParser;
use translator::parser::GameParser;

fn main() -> anyhow::Result<()> {
    println!("=== Testing RenpyParser on Real Game Files ===\n");

    let game_dir = Path::new(r"F:\Playnite\Games\0b588a07-1844-467f-8459-e5629642a1bf\Installed\game");
    let tl_dir = game_dir.join(r"tl\russian");

    if !tl_dir.exists() {
        println!("Game directory not found at {:?}", tl_dir);
        return Ok(());
    }

    // 1. Извлекаем спикеров из оригинальных скриптов игры
    println!("1. Scanning game scripts for Character definitions...");
    let start_speakers = Instant::now();
    let mut total_speakers = std::collections::HashMap::new();

    if let Ok(entries) = std::fs::read_dir(game_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("rpy") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let speakers = RenpyParser::extract_speakers(&content);
                    total_speakers.extend(speakers);
                }
            }
        }
    }
    println!("Found {} unique characters in {:?}", total_speakers.len(), start_speakers.elapsed());
    for (alias, name) in total_speakers.iter().take(8) {
        println!("   [{}] -> {}", alias, name);
    }
    if total_speakers.len() > 8 {
        println!("   ... and {} more", total_speakers.len() - 8);
    }

    let parser = RenpyParser::new().with_speakers(total_speakers);

    // 2. Тестируем парсинг реальных файлов перевода
    let test_files = ["script.rpy", "screens.rpy", "erika.rpy", "celia.rpy"];

    println!("\n2. Parsing translation files from game/tl/russian/...");
    for file_name in &test_files {
        let file_path = tl_dir.join(file_name);
        if !file_path.exists() {
            continue;
        }

        let content = match std::fs::read_to_string(&file_path) {
            Ok(c) => c,
            Err(e) => {
                println!("Failed to read {:?}: {}", file_path, e);
                continue;
            }
        };

        let start = Instant::now();
        let graph = parser.parse_file(&file_path, &content)?;
        let elapsed = start.elapsed();

        let (total, translated) = graph.translation_stats();
        let file_size_kb = content.len() as f64 / 1024.0;

        println!(
            "-> {:<14} ({:>6.1} KB) | {:>4} lines total | {:>4} translated ({:>5.1}%) | parsed in {:?}",
            file_name,
            file_size_kb,
            total,
            translated,
            if total > 0 { (translated as f64 / total as f64) * 100.0 } else { 0.0 },
            elapsed
        );
    }

    Ok(())
}
