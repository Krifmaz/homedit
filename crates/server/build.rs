//! Build script: makes sure the embedded UI folder exists so the crate builds before the frontend has been
//! built, and builds in the server settings (settings.toml → settings_gen.rs) and the release notes
//! (docs/releases/<version>.md → changelog_gen.rs) through the kit.

use kit_server_settings::file::{Codegen, generate};
use std::path::Path;

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dist = dir.join("../../web/dist");
    if !dist.exists() {
        std::fs::create_dir_all(&dist).expect("create web/dist");
    }
    println!("cargo:rerun-if-changed=../../web/dist");

    let out = std::env::var("OUT_DIR").expect("OUT_DIR is set in a build script");
    println!("cargo:rerun-if-changed=settings.toml");
    let text = std::fs::read_to_string(dir.join("settings.toml"))
        .expect("settings.toml is next to Cargo.toml");
    let code = generate(
        &text,
        &Codegen {
            accessors_on: None,
            methods: &[],
        },
    )
    .unwrap_or_else(|e| panic!("settings.toml: {e}"));
    std::fs::write(Path::new(&out).join("settings_gen.rs"), code).expect("OUT_DIR is writable");
    kit_server_changelog::build::embed(&dir.join("../../docs/releases"), "changelog_gen.rs");
}
