use std::env;
use std::path::PathBuf;
use std::fs;

fn main() {
    println!("cargo:rustc-link-arg-bins=--nmagic");
    println!("cargo:rustc-link-arg-bins=-Tlink.x");
    println!("cargo:rustc-link-arg-bins=-Tdefmt.x");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let dbc_path = manifest_dir.join("../exo-can/exo_can.dbc");
    let dbc_content = fs::read(&dbc_path).expect(&format!("Failed to read file: {:?}", dbc_path));

    println!("cargo:rerun-if-changed={}", dbc_path.display());

    let mut out = Vec::<u8>::new();
    dbc_codegen::codegen("exo_can.dbc", &dbc_content, &mut out, false).expect("dbc-codegen failed");

    let mut generated = String::from_utf8(out).unwrap();

    // Remove inner attributes (#![...]) and inner doc comments (//!)
    // These can't be used in included modules
    let lines: Vec<&str> = generated.lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.starts_with("#![") && !trimmed.starts_with("//!")
        })
        .collect();
    generated = lines.join("\n");

    fs::write(
        PathBuf::from(env::var("OUT_DIR").unwrap()).join("dbc_gen.rs"),
        generated,
    ).unwrap();
}
