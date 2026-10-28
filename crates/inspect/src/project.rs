//! A project root, and everything the inspector derives from it.
//!
//! `pliron-inspect --project <root>` points at a cargo workspace (or
//! package). Its `Cargo.toml` names, under `[workspace.metadata.pliron-inspect]`
//! (or `[package.metadata.pliron-inspect]`), the project's own binaries and
//! where its traces live:
//!
//! ```toml
//! [workspace.metadata.pliron-inspect]
//! driver = "crabbit-inspect-driver"   # one-shot render driver (CFG/tree views)
//! analysisd = "crabbit-analysisd"     # resident analysis server (runs, replay, costs)
//! traces = [".crabbit-traces"]        # trace dirs, relative to the root (the default)
//! ```
//!
//! The binaries are taken from the project's `target/` (built there with
//! `cargo build --bin <name>` when missing); the traces, emitted modules
//! and profiles are found under the trace directories; the dialect sources
//! are the project's own crates, reached through pliron-lsp (see
//! [`crate::lsp`]).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context;

/// `[workspace.metadata.pliron-inspect]`.
#[derive(Debug, Default, Clone, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct Metadata {
    /// Name of the one-shot render driver binary (`pliron_inspect_driver::run_stdio_driver`).
    pub driver: Option<String>,
    /// Name of the resident analysis server binary (`pliron_inspect_driver::run_server_stdio`).
    pub analysisd: Option<String>,
    /// Trace directories, relative to the root. Default: `.crabbit-traces`.
    pub traces: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Project {
    pub root: PathBuf,
    pub metadata: Metadata,
    /// Absolute trace directories (the first one receives imports).
    pub trace_dirs: Vec<PathBuf>,
    pub driver: Option<PathBuf>,
    pub analysisd: Option<PathBuf>,
}

pub const DEFAULT_TRACES: &str = ".crabbit-traces";

impl Project {
    /// Read the project at `root`; with `build`, binaries missing from its
    /// `target/` are built there.
    pub fn discover(root: &Path, build: bool) -> anyhow::Result<Project> {
        let root = std::fs::canonicalize(root)
            .with_context(|| format!("cannot resolve project root {}", root.display()))?;
        let manifest = root.join("Cargo.toml");
        anyhow::ensure!(
            manifest.is_file(),
            "{} is not a cargo project (no Cargo.toml)",
            root.display()
        );
        let metadata = read_metadata(&manifest)?;
        let traces: Vec<&str> = if metadata.traces.is_empty() {
            vec![DEFAULT_TRACES]
        } else {
            metadata.traces.iter().map(String::as_str).collect()
        };
        let trace_dirs = traces.iter().map(|t| root.join(t)).collect();
        let driver = match &metadata.driver {
            Some(name) => Some(find_or_build_bin(&root, name, build)?),
            None => None,
        };
        let analysisd = match &metadata.analysisd {
            Some(name) => Some(find_or_build_bin(&root, name, build)?),
            None => None,
        };
        Ok(Project {
            root,
            metadata,
            trace_dirs,
            driver,
            analysisd,
        })
    }

    /// Emitted IR modules (`.plir` / `.pliron`) under the trace directories.
    pub fn modules(&self) -> Vec<PathBuf> {
        self.find(|p| p.extension().is_some_and(|e| e == "plir" || e == "pliron"))
    }

    /// Profiles (`op_costs.json` payloads of `profile_ingest.py`) under the
    /// trace directories.
    pub fn profiles(&self) -> Vec<PathBuf> {
        self.find(|p| {
            p.extension().is_some_and(|e| e == "json")
                && p.file_name()
                    .is_some_and(|n| n.to_string_lossy().contains("op_costs"))
        })
    }

    /// Is `path` inside one of the trace directories? (What the UI may read.)
    pub fn owns(&self, path: &Path) -> bool {
        let Ok(path) = std::fs::canonicalize(path) else {
            return false;
        };
        self.trace_dirs
            .iter()
            .filter_map(|d| std::fs::canonicalize(d).ok())
            .any(|d| path.starts_with(d))
    }

    /// The virtual file of an IR document served by the UI (`module:<id>`,
    /// `run:<id>:<pass>`). It is never written; its location under the
    /// project makes pliron-lsp route it to the project's dialect engine.
    pub fn document_path(&self, key: &str) -> PathBuf {
        self.root
            .join("target")
            .join("pliron-inspect")
            .join(format!("{}.plir", key.replace(':', "-")))
    }

    fn find(&self, keep: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for dir in &self.trace_dirs {
            walk(dir, 0, &keep, &mut out);
        }
        out.sort();
        out
    }
}

fn walk(dir: &Path, depth: usize, keep: &impl Fn(&Path) -> bool, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth < 6 {
                walk(&p, depth + 1, keep, out);
            }
        } else if keep(&p) {
            out.push(p);
        }
    }
}

fn read_metadata(manifest: &Path) -> anyhow::Result<Metadata> {
    let text = std::fs::read_to_string(manifest)?;
    let value: toml::Value = text
        .parse()
        .with_context(|| format!("parsing {}", manifest.display()))?;
    let table = ["workspace", "package"].into_iter().find_map(|section| {
        value
            .get(section)?
            .get("metadata")?
            .get("pliron-inspect")
            .cloned()
    });
    match table {
        Some(t) => t
            .try_into()
            .context("[workspace.metadata.pliron-inspect] has unexpected fields"),
        None => Ok(Metadata::default()),
    }
}

/// The project's cargo target directory.
fn target_dir(root: &Path) -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"))
}

/// The newest built copy of the binary `name` under the project's target
/// directory, building it there when there is none (and `build`).
fn find_or_build_bin(root: &Path, name: &str, build: bool) -> anyhow::Result<PathBuf> {
    let exe = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    let target = target_dir(root);
    let candidates = [
        target.join("debug").join(&exe),
        target.join("release").join(&exe),
    ];
    let newest = || {
        candidates
            .iter()
            .filter(|p| p.is_file())
            .max_by_key(|p| p.metadata().and_then(|m| m.modified()).ok())
            .cloned()
    };
    if let Some(p) = newest() {
        return Ok(p);
    }
    anyhow::ensure!(
        build,
        "{name} is not built under {}: run `cargo build --bin {name}` there",
        target.display()
    );
    eprintln!(
        "pliron-inspect: building {name} in {} (cargo build --bin {name})",
        root.display()
    );
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = Command::new(cargo)
        .current_dir(root)
        .args(["build", "--bin", name])
        .status()
        .with_context(|| format!("running cargo build --bin {name}"))?;
    anyhow::ensure!(
        status.success(),
        "cargo build --bin {name} failed in {}",
        root.display()
    );
    newest().ok_or_else(|| {
        anyhow::anyhow!(
            "cargo build --bin {name} succeeded but {} is missing",
            candidates[0].display()
        )
    })
}

/// The `pliron-lsp` executable (github.com/montaglue/pliron-lsp):
/// `$PLIRON_LSP`, next to this executable, `~/.cargo/bin`, then `PATH`.
pub fn find_pliron_lsp() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("PLIRON_LSP") {
        return Some(PathBuf::from(p));
    }
    let name = format!("pliron-lsp{}", std::env::consts::EXE_SUFFIX);
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join(&name));
    }
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".cargo").join("bin").join(&name));
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            candidates.push(dir.join(&name));
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_from_workspace_or_package() {
        let dir =
            std::env::temp_dir().join(format!("pliron-inspect-project-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = dir.join("Cargo.toml");
        std::fs::write(
            &manifest,
            "[workspace]\nmembers = []\n[workspace.metadata.pliron-inspect]\nanalysisd = \"x-analysisd\"\ntraces = [\"traces\", \"more\"]\n",
        )
        .unwrap();
        let m = read_metadata(&manifest).unwrap();
        assert_eq!(m.analysisd.as_deref(), Some("x-analysisd"));
        assert_eq!(m.driver, None);
        assert_eq!(m.traces, ["traces", "more"]);
        let p = Project::discover(&dir, false);
        // The binary is not built: discovery says so instead of building.
        assert!(p.unwrap_err().to_string().contains("x-analysisd"));

        std::fs::write(&manifest, "[package]\nname = \"p\"\nversion = \"0.0.0\"\n").unwrap();
        let p = Project::discover(&dir, false).unwrap();
        assert!(p.trace_dirs[0].ends_with(DEFAULT_TRACES));
        assert!(
            p.document_path("run:3:7")
                .ends_with("target/pliron-inspect/run-3-7.plir")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
