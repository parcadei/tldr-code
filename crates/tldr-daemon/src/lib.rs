//! TLDR Daemon library - Background service for code analysis
//!
//! Re-exports daemon internals for use by the `tldr-cli` bundled binary.

pub mod handlers;
pub mod server;
pub mod state;

pub use server::DaemonConfig;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use tracing::info;
use tracing_subscriber::EnvFilter;

use server::{compute_socket_path, compute_tcp_port};
use state::DaemonState;

/// TLDR Daemon - Background service for code analysis
#[derive(Parser, Debug)]
#[command(name = "tldr-daemon")]
#[command(version, about, long_about = None)]
struct Args {
    /// Project root directory (default: current directory)
    #[arg(short, long)]
    project: Option<PathBuf>,

    /// Idle timeout in seconds before auto-shutdown (default: 300)
    #[arg(long, default_value = "300")]
    idle_timeout: u64,

    /// Use TCP instead of Unix socket
    #[arg(long)]
    tcp: bool,

    /// TCP port (only with --tcp, default: computed from project hash)
    #[arg(long)]
    port: Option<u16>,

    /// Show status and exit (don't start daemon)
    #[arg(long)]
    status: bool,

    /// Stop running daemon and exit
    #[arg(long)]
    stop: bool,
}

/// Run the daemon (blocking). Entry point for both standalone and bundled binaries.
pub fn run() -> anyhow::Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async_main())
}

async fn async_main() -> anyhow::Result<()> {
    let filter = EnvFilter::try_from_env("TLDR_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    let args = Args::parse();

    let project = args
        .project
        .map(|p| dunce::canonicalize(&p).unwrap_or(p))
        .unwrap_or_else(|| {
            std::env::current_dir()
                .map(|p| dunce::canonicalize(&p).unwrap_or(p))
                .unwrap_or_else(|_| PathBuf::from("."))
        });

    info!("Project root: {:?}", project);

    let socket_path = compute_socket_path(&project, "1.0");
    info!("Socket path: {:?}", socket_path);

    if args.status {
        return handle_status(&socket_path).await;
    }

    if args.stop {
        return handle_stop(&socket_path).await;
    }

    let state = Arc::new(
        DaemonState::new(project.clone(), socket_path.clone())
            .with_idle_timeout(Duration::from_secs(args.idle_timeout)),
    );

    #[cfg(unix)]
    if !args.tcp {
        info!("Starting daemon on Unix socket");
        server::run_unix_socket(&socket_path, state).await?;
    } else {
        let port = args.port.unwrap_or_else(|| compute_tcp_port(&project));
        let addr: SocketAddr = format!("127.0.0.1:{}", port).parse()?;
        info!("Starting daemon on TCP {}", addr);
        server::run_tcp(addr, state).await?;
    }

    #[cfg(windows)]
    {
        let port = args.port.unwrap_or_else(|| compute_tcp_port(&project));
        let addr: SocketAddr = format!("127.0.0.1:{}", port).parse()?;
        info!("Starting daemon on TCP {}", addr);
        server::run_tcp(addr, state).await?;
    }

    Ok(())
}

async fn handle_status(socket_path: &PathBuf) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        match tokio::net::UnixStream::connect(socket_path).await {
            Ok(mut stream) => {
                let request = r#"{"cmd":"status"}"#;
                stream
                    .write_all(
                        format!(
                            "POST /status HTTP/1.1\r\nContent-Length: {}\r\n\r\n{}",
                            request.len(),
                            request
                        )
                        .as_bytes(),
                    )
                    .await?;

                let mut buf = vec![0u8; 8192];
                let n = stream.read(&mut buf).await?;
                let response = String::from_utf8_lossy(&buf[..n]);

                if let Some(body_start) = response.find("\r\n\r\n") {
                    println!("{}", &response[body_start + 4..]);
                } else {
                    println!("{}", response);
                }
                Ok(())
            }
            Err(e) => {
                eprintln!("Daemon not running: {}", e);
                std::process::exit(1);
            }
        }
    }

    #[cfg(windows)]
    {
        let _ = socket_path;
        eprintln!("Status check not yet implemented for Windows TCP");
        std::process::exit(1);
    }
}

async fn handle_stop(socket_path: &PathBuf) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        if socket_path.exists() {
            match tokio::net::UnixStream::connect(socket_path).await {
                Ok(_) => {
                    std::fs::remove_file(socket_path)?;
                    println!("Daemon stopped (socket removed)");
                }
                Err(_) => {
                    std::fs::remove_file(socket_path)?;
                    println!("Removed stale socket");
                }
            }
        } else {
            println!("Daemon not running");
        }
        Ok(())
    }

    #[cfg(windows)]
    {
        let _ = socket_path;
        eprintln!("Stop not yet implemented for Windows TCP");
        std::process::exit(1);
    }
}
