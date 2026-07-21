mod driver;
mod server;

use clap::Parser as ClapParser;
use std::path::PathBuf;

use server::{AppState, build_router};

#[derive(ClapParser)]
#[command(name = "stair-display")]
#[command(about = "Interactive STAIR IR display")]
struct Args {
    /// Port to run the server on.
    #[arg(short, long, default_value = "3000")]
    port: u16,

    /// Do not open browser automatically.
    #[arg(long)]
    no_open: bool,

    /// Driver binary path for future CLI-backed render requests.
    #[arg(long)]
    driver: Option<String>,

    /// Mutable directory containing imported .stx STAIR event log files.
    ///
    /// Defaults to $STAIR_DISPLAY_TRACE_DIR, then ~/.stair/traces, then
    /// .stair-traces. $STAIR_DISPLAY_TRACE_DIR may contain multiple directories
    /// separated by ';'.
    #[arg(long)]
    trace_dir: Option<PathBuf>,

    /// Immutable temporary directory where compiler runs write .stx files.
    ///
    /// Defaults to $STAIR_DISPLAY_TEMP_TRACE_DIR, then STAIR's default temp
    /// trace directory. $STAIR_DISPLAY_TEMP_TRACE_DIR may contain multiple
    /// directories separated by ';'.
    #[arg(long)]
    temp_trace_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let trace_library_dirs = dirs_from_cli_env_default(
        args.trace_dir,
        "STAIR_DISPLAY_TRACE_DIR",
        default_trace_library_dirs(),
    );
    let temp_trace_dirs = dirs_from_cli_env_default(
        args.temp_trace_dir,
        "STAIR_DISPLAY_TEMP_TRACE_DIR",
        vec![PathBuf::from(pliron_inspect_protocol::trace::DEFAULT_TRACE_DIR)],
    );
    let state = AppState {
        driver_binary: args.driver.or_else(default_driver_binary),
        trace_library_dirs,
        temp_trace_dirs,
    };

    let app = build_router(state);
    let addr = format!("127.0.0.1:{}", args.port);

    if !args.no_open {
        let url = format!("http://{}", addr);
        eprintln!("Opening browser at {}", url);
        let _ = open::that(&url);
    }

    eprintln!("Server listening on http://{}", addr);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

fn default_trace_library_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".stair").join("traces"))
}

fn dirs_from_cli_env_default(
    cli_dir: Option<PathBuf>,
    env_name: &str,
    default_dirs: Vec<PathBuf>,
) -> Vec<PathBuf> {
    if let Some(cli_dir) = cli_dir {
        return vec![cli_dir];
    }

    let env_dirs = env_dirs(env_name);
    if env_dirs.is_empty() {
        default_dirs
    } else {
        env_dirs
    }
}

fn default_trace_library_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    dirs.push(default_trace_library_dir().unwrap_or_else(|| PathBuf::from(".stair-traces")));
    dirs.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("traces"),
    );
    dirs
}

fn env_dirs(name: &str) -> Vec<PathBuf> {
    let Some(value) = std::env::var_os(name) else {
        return vec![];
    };
    value
        .to_string_lossy()
        .split(';')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(PathBuf::from)
        .collect()
}

fn default_driver_binary() -> Option<String> {
    let current_exe = std::env::current_exe().ok()?;
    let exe_name = format!("pliron-inspect-driver{}", std::env::consts::EXE_SUFFIX);

    let sibling = current_exe.with_file_name(&exe_name);
    if sibling.exists() {
        return Some(sibling.to_string_lossy().to_string());
    }

    let workspace_target = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("debug")
        .join(&exe_name);
    if workspace_target.exists() {
        return Some(workspace_target.to_string_lossy().to_string());
    }

    Some("pliron-inspect-driver".to_string())
}
