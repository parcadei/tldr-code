//! TLDR Daemon - Background service for code analysis

fn main() -> anyhow::Result<()> {
    tldr_daemon::run()
}
