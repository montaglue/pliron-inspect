mod folders;
mod history;
mod line_diff;
mod lsp;
mod render;
mod server;
mod session;

use clap::Parser as ClapParser;
use std::path::PathBuf;

use server::{AppState, build_router};

#[derive(ClapParser)]
#[command(name = "pliron-inspect")]
#[command(about = "Interactive inspector for pliron compiler traces")]
struct Args {
    /// Port to run the server on.
    #[arg(short, long, default_value = "3000")]
    port: u16,

    /// Do not open browser automatically.
    #[arg(long)]
    no_open: bool,

    /// Listen on all network interfaces instead of 127.0.0.1 only. There is
    /// no authentication: anyone who can reach the port can open a project
    /// (building and running its binaries) and read files under it.
    #[arg(long)]
    listen_all: bool,

    /// Directory with the built frontend (index.html + assets). Defaults
    /// to the checkout's frontend/dist.
    #[arg(long)]
    frontend_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if std::env::args().nth(1).as_deref() == Some(render::cfg::LAYOUT_WORKER_FLAG) {
        render::cfg::layout_worker();
    }
    let args = Args::parse();
    let state = AppState {
        frontend_dir: args.frontend_dir,
        session: session::Session::new(),
        histories: Default::default(),
    };

    let app = build_router(state);
    let host = if args.listen_all {
        "0.0.0.0"
    } else {
        "127.0.0.1"
    };
    let addr = format!("{}:{}", host, args.port);
    let local = format!("127.0.0.1:{}", args.port);

    if !args.no_open {
        let url = format!("http://{}", local);
        eprintln!("Opening browser at {}", url);
        let _ = open::that(&url);
    }

    eprintln!("Server listening on http://{}", addr);
    if args.listen_all {
        eprintln!(
            "Listening on all interfaces without authentication: anyone who can reach port {} can open projects on this machine",
            args.port
        );
    }
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
