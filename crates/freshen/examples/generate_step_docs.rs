//! Generates one Markdown page per step into `docs/steps/`, from the step
//! registry.
//!
//! Run from the repository root:
//!
//! ```sh
//! cargo run --example generate_step_docs
//! ```
//!
//! The generated pages are committed to `docs/steps/` and listed in
//! `mkdocs.yml`. Re-run this whenever a step is added, renamed or its
//! platforms/prerequisites change so the docs stay in sync with the registry.

use std::fs;
use std::path::Path;

use freshen_lib::{builtin_steps, docs::pages, docs::render, registry::Registry};

const OUT_DIR: &str = "docs/steps";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let registry = builtin_steps::register_all(Registry::new(), None);
    let out = Path::new(OUT_DIR);
    fs::create_dir_all(out)?;

    let mut count = 0;
    for page in pages(&registry) {
        let markdown = render(&page);
        let path = out.join(format!("{}.md", page.id));
        fs::write(&path, markdown)?;
        count += 1;
        println!("wrote {}", path.display());
    }

    println!("generated {count} step pages in {OUT_DIR}");
    Ok(())
}
