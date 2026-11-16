//! Picks the built-in evaluation weights for each rule: `src/weights-RULE.txt`
//! when it exists (written by the Tune workflow), else the freestyle
//! `src/weights.txt`.

use std::path::PathBuf;

fn main() {
    let src = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir")).join("src");
    // Adding a weights file must rebuild; a directory is checked recursively.
    println!("cargo:rerun-if-changed=src");
    let rules = [
        ("Freestyle", "freestyle"),
        ("Standard", "standard"),
        ("Renju", "renju"),
        ("FreestyleCaro", "freestyle-caro"),
        ("Caro", "caro"),
    ];
    let mut code = String::from(
        "/// Text of the built-in weights for `rule`.\nfn built_in_text(rule: Rule) -> &'static str {\n    match rule {\n",
    );
    for (variant, name) in rules {
        let own = format!("weights-{name}.txt");
        let file = if src.join(&own).exists() {
            own
        } else {
            "weights.txt".to_string()
        };
        code.push_str(&format!(
            "        Rule::{variant} => include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/src/{file}\")),\n"
        ));
    }
    code.push_str("    }\n}\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("out dir")).join("weights.rs");
    std::fs::write(out, code).expect("write weights.rs");
}
