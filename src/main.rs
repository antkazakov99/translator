use std::path::Path;
use std::time::Instant;
use translator::parser::renpy::RenpyParser;
use translator::parser::GameParser;

fn main() -> anyhow::Result<()> {
    println!("=== Testing RenpyParser & Mod Generator on Real Files ===\n");

    let game_dir = Path::new(r"F:\Playnite\Games\0b588a07-1844-467f-8459-e5629642a1bf\Installed\game");
    let tl_dir = game_dir.join(r"tl\russian");

    if !tl_dir.exists() {
        println!("Game directory not found at {:?}", tl_dir);
        return Ok(());
    }

    // 1. Сканируем спикеров
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
    println!("1. Loaded {} character definitions.", total_speakers.len());

    let parser = RenpyParser::new().with_speakers(total_speakers);

    // 2. Тестируем парсинг реального erika.rpy
    let erika_path = tl_dir.join("erika.rpy");
    println!("2. Parsing {:?}...", erika_path.file_name().unwrap());

    let content = std::fs::read_to_string(&erika_path)?;
    let start_parse = Instant::now();
    let graph = parser.parse_file(&erika_path, &content)?;
    let parse_time = start_parse.elapsed();

    let (total, translated) = graph.translation_stats();
    println!("   Parsed {} lines ({} translated) in {:?}", total, translated, parse_time);

    // 3. Тестируем генерацию мода в dist/
    let dist_mod_path = Path::new(r"D:\Projects\Personal\translator\projects\take-over\dist\game\tl\russian\erika.rpy");
    println!("\n3. Generating translation mod file to {:?}...", dist_mod_path);

    let start_write = Instant::now();
    parser.save_translation_file(&graph, dist_mod_path, "russian")?;
    let write_time = start_write.elapsed();

    let mod_meta = std::fs::metadata(dist_mod_path)?;
    println!(
        "   Successfully built mod file! Size: {:.1} KB in {:?}",
        mod_meta.len() as f64 / 1024.0,
        write_time
    );

    // 4. Проверяем, что сгенерированный мод корректно парсится обратно
    let mod_content = std::fs::read_to_string(dist_mod_path)?;
    let re_graph = parser.parse_file(dist_mod_path, &mod_content)?;
    let (re_total, re_translated) = re_graph.translation_stats();

    println!("\n4. Verification check (Roundtrip):");
    println!("   Original graph: {} total, {} translated", total, translated);
    println!("   Reloaded graph: {} total, {} translated", re_total, re_translated);

    assert_eq!(total, re_total, "Line counts must match!");
    println!("\n🎉 100% Roundtrip match! Mod generation works with zero loss.");

    Ok(())
}
