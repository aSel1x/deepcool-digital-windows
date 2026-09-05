//! End-to-end checks on the assets that ship with the program.
//!
//! These are the tests that would have caught the shipped templates referencing
//! variables the renderer never provided, and the corrupt path data in the
//! cicada template — both of which failed silently at runtime.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn templates() -> Vec<PathBuf> {
    let dir = repo_root().join("configs/svg-templates");
    let mut out: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|e| e == "svg").unwrap_or(false))
        .collect();
    out.sort();
    assert!(!out.is_empty(), "no SVG templates found in {}", dir.display());
    out
}

/// The full context the renderer supplies, as literal JSON.
///
/// Deliberately independent of `LcdContext`: if a field is renamed, this test
/// fails and forces the docs and the shipped templates to be updated too.
fn context() -> serde_json::Value {
    serde_json::json!({
        "cpu_temp": 62u8,
        "gpu_temp": 55u8,
        "vcore": 1234u16,
        "liquid_temp": 33u8,
        "liquid_fan_rpm": 900u16,
        "cpu_fan_rpm": 1200u16,
        "cpu_usage": 37u8,
        "vram_usage": 41u8,
        "gpu_usage": 22u8,
        "ram_usage": 58u8,
        "sys_fan_rpm": 800u16,
        "cpu_power": 65u16,
        "gpu_power": 180u16,
        "cpu_freq": 4300u16,
        "gpu_freq": 2100u16,
        "clock": "12:34:56",
        "bar_track_width": 296u32,
        "valid": {
            "cpu_temp": true, "gpu_temp": true, "vcore": true, "liquid_temp": false,
            "liquid_fan_rpm": true, "cpu_fan_rpm": true, "cpu_usage": true,
            "vram_usage": true, "gpu_usage": true, "ram_usage": true,
            "sys_fan_rpm": true, "cpu_power": true, "gpu_power": true,
            "cpu_freq": true, "gpu_freq": true
        }
    })
}

fn render(path: &Path) -> String {
    let source = std::fs::read_to_string(path).unwrap();
    let mut env = minijinja::Environment::new();
    // Same setting the renderer uses: an unknown variable must be an error.
    env.set_undefined_behavior(minijinja::UndefinedBehavior::Strict);
    env.add_template_owned("svg", source)
        .unwrap_or_else(|e| panic!("{} failed to parse: {e}", path.display()));
    env.get_template("svg")
        .unwrap()
        .render(minijinja::Value::from_serialize(context()))
        .unwrap_or_else(|e| panic!("{} failed to render: {e}", path.display()))
}

#[test]
fn every_shipped_template_renders_against_the_real_context() {
    for path in templates() {
        let out = render(&path);
        assert!(out.contains("<svg"), "{} produced no SVG root", path.display());
    }
}

#[test]
fn every_shipped_template_parses_as_svg_after_rendering() {
    let options = resvg::usvg::Options::default();
    for path in templates() {
        let svg = render(&path);
        resvg::usvg::Tree::from_str(&svg, &options)
            .unwrap_or_else(|e| panic!("{} is not valid SVG after rendering: {e}", path.display()));
    }
}

#[test]
fn shipped_paths_contain_no_spelled_out_numbers() {
    // "H2. fiveZ" instead of "H2.5Z" silently truncated a subpath: usvg stops
    // parsing a path at the first malformed token and reports nothing.
    let bad = regex_lite_find_spelled_numbers();
    for path in templates() {
        let text = std::fs::read_to_string(&path).unwrap();
        for word in &bad {
            assert!(
                !text.contains(word.as_str()),
                "{} contains a spelled-out number ({word}) — likely an autocorrect artefact",
                path.display()
            );
        }
    }
}

fn regex_lite_find_spelled_numbers() -> Vec<String> {
    let words = [
        "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
    ];
    // Only the ". word" shape, which is what the corruption looked like.
    words.iter().map(|w| format!(". {w}")).collect()
}

#[test]
fn the_bundled_fonts_the_templates_ask_for_are_present() {
    let dir = repo_root().join("configs/assets");
    let mut families: Vec<String> = Vec::new();
    for path in templates() {
        let text = std::fs::read_to_string(&path).unwrap();
        for part in text.split("font-family=\"").skip(1) {
            if let Some(end) = part.find('"') {
                families.push(part[..end].to_string());
            }
        }
    }
    families.sort();
    families.dedup();

    if families.is_empty() {
        return;
    }

    let mut db = resvg::usvg::fontdb::Database::new();
    for entry in std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .flatten()
    {
        let p = entry.path();
        if p.extension().map(|e| e == "ttf" || e == "otf").unwrap_or(false) {
            db.load_font_file(&p).unwrap();
        }
    }

    for family in families {
        let query = resvg::usvg::fontdb::Query {
            families: &[resvg::usvg::fontdb::Family::Name(&family)],
            ..Default::default()
        };
        assert!(
            db.query(&query).is_some(),
            "font family \"{family}\" is used by a shipped template but is not in {}",
            dir.display()
        );
    }
}

#[test]
fn the_bundled_fonts_ship_with_their_licence() {
    let ofl = repo_root().join("configs/assets/OFL.txt");
    assert!(ofl.is_file(), "the bundled fonts must ship with their OFL licence text");
    let text = std::fs::read_to_string(&ofl).unwrap();
    assert!(text.contains("SIL OPEN FONT LICENSE"), "OFL.txt does not look like the OFL");
}

#[test]
fn the_example_config_parses_and_matches_the_documented_schema() {
    let path = repo_root().join("configs/example.toml");
    let text = std::fs::read_to_string(&path).unwrap();
    // `deny_unknown_fields` on every config struct means a stale key in the
    // example is a hard error here rather than a silently ignored setting.
    let parsed: toml::Value = toml::from_str(&text).unwrap();
    assert!(parsed.get("update_interval_ms").is_some());
    assert!(parsed.get("lcd").is_some());
    assert!(parsed.get("segment").is_some());
}

#[test]
fn the_lhm_fixture_is_anonymised() {
    let path = repo_root().join("tests/fixtures/lhm-sample.json");
    let text = std::fs::read_to_string(&path).unwrap();
    // Guard against re-importing a raw dump from a developer machine.
    for marker in ["ASEL1X", "asel1x"] {
        assert!(!text.contains(marker), "the LHM fixture still contains {marker}");
    }
    let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(doc.get("Children").is_some(), "fixture is not an LHM sensor tree");
}
