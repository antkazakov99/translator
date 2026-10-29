use translator::project::{EngineType, ProjectManifest};
use translator::graph::ScenarioGraph;

fn main() {
    println!("=== Game Localization & Translation Engine ===");
    let manifest = ProjectManifest::new(
        "sample",
        "Sample Visual Novel",
        EngineType::Renpy,
        "F:/Games/Sample".into(),
        "russian",
    );
    println!("Initialized project: {} ({:?})", manifest.name, manifest.engine);

    let graph = ScenarioGraph::new();
    let (total, translated) = graph.translation_stats();
    println!("Graph initialized. Lines: {}/{} translated", translated, total);
}
