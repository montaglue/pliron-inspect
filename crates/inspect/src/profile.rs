//! The profile a trace carries (backward PGO, in its header's `bpgo`): for
//! each snapshot it reaches, the metrics of each op, by the line the op
//! starts on; and, if it was measured with them, the cycle samples one by
//! one (`timeline`). Read once per trace.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use pliron_inspect_protocol::trace::CrabbitTraceFile;
use tokio::sync::OnceCell;

use crate::render::flame::LineProfile;

/// A trace's profile.
#[derive(Default)]
struct TraceProfile {
    /// Snapshot index → its ops' metrics by line.
    snapshots: HashMap<usize, Arc<LineProfile>>,
    timeline: Option<Arc<TimelineSamples>>,
}

/// The cycle samples of a run, one by one, on the ops of one snapshot (the
/// trace's header names which).
pub struct TimelineSamples {
    /// Each sample's time after the one before (ns).
    pub time: Vec<u64>,
    /// The line of the op each sample hit (0: outside the module).
    pub line: Vec<u32>,
}

/// The profiles of the traces asked for, read once per trace.
#[derive(Default)]
pub struct Profiles {
    traces: Mutex<HashMap<PathBuf, Arc<OnceCell<Arc<TraceProfile>>>>>,
}

impl Profiles {
    /// The profile of snapshot `step` of the trace at `path` (canonical);
    /// `None` if the trace has no profile or it does not reach the snapshot.
    pub async fn snapshot(&self, path: &Path, step: usize) -> Result<Option<Arc<LineProfile>>> {
        Ok(self.trace(path).await?.snapshots.get(&step).cloned())
    }

    /// The trace's samples one by one, if it was measured with them.
    pub async fn timeline(&self, path: &Path) -> Result<Option<Arc<TimelineSamples>>> {
        Ok(self.trace(path).await?.timeline.clone())
    }

    async fn trace(&self, path: &Path) -> Result<Arc<TraceProfile>> {
        let cell = self
            .traces
            .lock()
            .unwrap()
            .entry(path.to_path_buf())
            .or_default()
            .clone();
        let path = path.to_path_buf();
        cell.get_or_try_init(|| async move {
            tokio::task::spawn_blocking(move || read(&path))
                .await
                .context("reading the profile panicked")?
                .map(Arc::new)
        })
        .await
        .cloned()
    }
}

/// `bpgo.snapshots` of the trace's header: `{"<index>": [[line, column,
/// {metric: value}], ...]}`; ops starting on one line add up.
fn read(path: &Path) -> Result<TraceProfile> {
    let trace = CrabbitTraceFile::read(path)?;
    let mut profile = TraceProfile::default();
    let Some(bpgo) = trace.meta.extra.get("bpgo") else {
        return Ok(profile);
    };
    profile.timeline = timeline(bpgo).map(Arc::new);
    let Some(snapshots) = bpgo
        .get("snapshots")
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
        profile.snapshots.insert(index, Arc::new(lines));
    }
    Ok(profile)
}

/// `bpgo.timeline`: `{"time": [ns after the previous], "line": [...]}`.
fn timeline(bpgo: &serde_json::Value) -> Option<TimelineSamples> {
    let timeline = bpgo.get("timeline")?;
    let numbers = |key: &str| -> Option<Vec<u64>> {
        timeline
            .get(key)?
            .as_array()?
            .iter()
            .map(|v| v.as_u64())
            .collect()
    };
    let time = numbers("time")?;
    let line: Vec<u32> = numbers("line")?.into_iter().map(|l| l as u32).collect();
    (time.len() == line.len()).then_some(TimelineSamples { time, line })
}
