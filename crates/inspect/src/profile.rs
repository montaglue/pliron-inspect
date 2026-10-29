//! The profile a trace carries (backward PGO, in its header's `bpgo`): for
//! each snapshot it reaches, the metrics of each op, by the line the op
//! starts on. Read once per trace.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use pliron_inspect_protocol::trace::CrabbitTraceFile;
use tokio::sync::OnceCell;

use crate::render::flame::LineProfile;

/// A trace's profile: snapshot index → its ops' metrics by line.
type TraceProfile = HashMap<usize, Arc<LineProfile>>;

/// The profiles of the traces asked for, read once per trace.
#[derive(Default)]
pub struct Profiles {
    traces: Mutex<HashMap<PathBuf, Arc<OnceCell<Arc<TraceProfile>>>>>,
}

impl Profiles {
    /// The profile of snapshot `step` of the trace at `path` (canonical);
    /// `None` if the trace has no profile or it does not reach the snapshot.
    pub async fn snapshot(&self, path: &Path, step: usize) -> Result<Option<Arc<LineProfile>>> {
        let cell = self
            .traces
            .lock()
            .unwrap()
            .entry(path.to_path_buf())
            .or_default()
            .clone();
        let path = path.to_path_buf();
        let profile = cell
            .get_or_try_init(|| async move {
                tokio::task::spawn_blocking(move || read(&path))
                    .await
                    .context("reading the profile panicked")?
                    .map(Arc::new)
            })
            .await?;
        Ok(profile.get(&step).cloned())
    }
}

/// `bpgo.snapshots` of the trace's header: `{"<index>": [[line, column,
/// {metric: value}], ...]}`; ops starting on one line add up.
fn read(path: &Path) -> Result<TraceProfile> {
    let trace = CrabbitTraceFile::read(path)?;
    let mut profile = TraceProfile::new();
    let Some(snapshots) = trace
        .meta
        .extra
        .get("bpgo")
        .and_then(|bpgo| bpgo.get("snapshots"))
        .and_then(|snapshots| snapshots.as_object())
    else {
        return Ok(profile);
    };
    for (index, ops) in snapshots {
        let Ok(index) = index.parse::<usize>() else {
            continue;
        };
        let mut lines = LineProfile::new();
        for op in ops.as_array().into_iter().flatten() {
            let (Some(line), Some(metrics)) = (
                op.get(0).and_then(|line| line.as_u64()),
                op.get(2).and_then(|metrics| metrics.as_object()),
            ) else {
                continue;
            };
            let entry = lines.entry(line as u32).or_default();
            for (name, value) in metrics {
                if let Some(value) = value.as_f64() {
                    *entry.entry(name.clone()).or_insert(0.0) += value;
                }
            }
        }
        profile.insert(index, Arc::new(lines));
    }
    Ok(profile)
}
