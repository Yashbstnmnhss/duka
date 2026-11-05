//! A guard on how a `Type` is inspected.
//!
//! Adding a variant to `Type` compiles everywhere: a `matches!` or an `if let`
//! that never expected the new shape keeps compiling and quietly answers
//! `false`. The checker is written in that style, so a type that carries a
//! label or a pattern has to be read through `Type::base()` at the places that
//! only ask what kind of thing it is.
//!
//! This does not fix the sites that exist. It stops new ones from arriving,
//! and the number may only go down.

use std::{
    fs,
    path::{Path, PathBuf},
};

/// How many sites inspect a `Type` without asking `base()` first.
const BASELINE: usize = 99;

const SKIP: &[&str] = &["target", "ffi", "shared", "website", "node_modules", ".git"];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if SKIP.contains(&name.as_str()) {
                continue;
            }
            // this test lives here, and its own text is full of the pattern
            if name == "tests" && path.parent().map(|p| p.to_string_lossy() == *"lib") == Some(true)
            {
                continue;
            }
            rust_files(&path, out);
        } else if name.ends_with(".rs") {
            out.push(path);
        }
    }
}

fn inspects(line: &str) -> bool {
    line.contains("Type::")
        && (line.contains("matches!(") || line.contains("if let ") || line.contains("while let "))
}

#[test]
fn type_inspection_does_not_grow() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the crate sits in the workspace");
    let mut files = Vec::new();
    rust_files(root, &mut files);
    let mut hits = Vec::new();
    for file in files {
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        for (index, line) in text.lines().enumerate() {
            if inspects(line) {
                hits.push(format!("{}:{}", file.display(), index + 1));
            }
        }
    }
    assert!(
        hits.len() <= BASELINE,
        "read the type through `Type::base()` first: {} sites, baseline {}\n{}",
        hits.len(),
        BASELINE,
        hits.join("\n")
    );
}
