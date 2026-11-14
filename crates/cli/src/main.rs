//! The lsdocs command-line interface.

// Placeholder uses so the intended dependency graph builds cleanly;
// remove each one once real code imports the crate.
use lsdocs_config as _;
use lsdocs_host as _;
use lsdocs_parser_markdown as _;
use lsdocs_render_html as _;
use lsdocs_transform_toc as _;

fn main() {
    println!("lsdocs {}", env!("CARGO_PKG_VERSION"));
}
