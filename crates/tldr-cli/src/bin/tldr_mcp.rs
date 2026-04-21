//! Thin wrapper that re-exports the tldr-mcp binary.
//! This exists so cargo-dist bundles all TLDR binaries into a single archive.

fn main() {
    tldr_mcp::run();
}
