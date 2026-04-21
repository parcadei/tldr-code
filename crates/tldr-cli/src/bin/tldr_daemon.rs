//! Thin wrapper that re-exports the tldr-daemon binary.
//! This exists so cargo-dist bundles all TLDR binaries into a single archive.

fn main() -> anyhow::Result<()> {
    tldr_daemon::run()
}
