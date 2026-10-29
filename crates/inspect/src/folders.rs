//! The folder browser behind the UI's folder pickers. The browser may run
//! on another machine, so its own file dialog cannot pick the server's
//! folders; the server lists them instead, under the user's home directory
//! only, with what each folder looks like to the pickers.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// A folder and its subfolders.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub path: PathBuf,
    /// `None` at the home directory.
    pub parent: Option<PathBuf>,
    pub home: PathBuf,
    pub marks: Marks,
    pub folders: Vec<Folder>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub name: String,
    pub path: PathBuf,
    pub marks: Marks,
}

/// What a folder holds, as far as the pickers care.
#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Marks {
    /// It has a `Cargo.toml`.
    pub cargo: bool,
    /// Its `Cargo.toml` mentions pliron: a project with pliron dialects.
    pub pliron: bool,
    /// Traces (`.crt`) in it or in its subfolders.
    pub traces: usize,
}

/// The folder at `path` (default: the home directory), which must be under
/// the home directory.
pub fn list(path: Option<&str>) -> Result<Listing, String> {
    let home = home()?;
    let path = match path.filter(|p| !p.is_empty()) {
        Some(p) => resolve(p)?,
        None => home.clone(),
    };
    let mut folders: Vec<Folder> = std::fs::read_dir(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .map(|p| Folder {
            name: p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            marks: marks(&p),
            path: p,
        })
        .collect();
    folders.sort_by_key(|f| (f.name.starts_with('.'), f.name.to_lowercase()));
    let parent = if path == home {
        None
    } else {
        path.parent().map(Path::to_path_buf)
    };
    Ok(Listing {
        marks: marks(&path),
        path,
        parent,
        home,
        folders,
    })
}

/// `path` (`~/` expanded) as an existing folder under the home directory.
pub fn resolve(path: &str) -> Result<PathBuf, String> {
    let home = home()?;
    let expanded = match path.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if path == "~" => home.clone(),
        None => PathBuf::from(path),
    };
    let resolved = std::fs::canonicalize(&expanded).map_err(|e| format!("{path}: {e}"))?;
    if !resolved.starts_with(&home) {
        return Err(format!(
            "{} is outside the home directory",
            resolved.display()
        ));
    }
    if !resolved.is_dir() {
        return Err(format!("{} is not a folder", resolved.display()));
    }
    Ok(resolved)
}

fn home() -> Result<PathBuf, String> {
    dirs::home_dir()
        .and_then(|h| std::fs::canonicalize(h).ok())
        .ok_or_else(|| "no home directory".to_string())
}

/// Subfolders read per folder when counting traces.
const SUBFOLDER_LIMIT: usize = 200;

fn marks(dir: &Path) -> Marks {
    let manifest = dir.join("Cargo.toml");
    let cargo = manifest.is_file();
    let pliron =
        cargo && std::fs::read_to_string(&manifest).is_ok_and(|text| text.contains("pliron"));
    let mut traces = count(dir, &["crt"]);
    if let Ok(entries) = std::fs::read_dir(dir) {
        for sub in entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .take(SUBFOLDER_LIMIT)
        {
            traces += count(&sub, &["crt"]);
        }
    }
    Marks {
        cargo,
        pliron,
        traces,
    }
}

fn count(dir: &Path, extensions: &[&str]) -> usize {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| {
                    e.path()
                        .extension()
                        .is_some_and(|x| extensions.iter().any(|want| x == *want))
                })
                .count()
        })
        .unwrap_or(0)
}
