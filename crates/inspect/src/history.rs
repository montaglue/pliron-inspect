//! The diff of every pass of a trace and the blame of every snapshot,
//! computed once and saved.
//!
//! Step `i` compares snapshot `i - 1` with snapshot `i`, line by line, as
//! printed. Its hunks are what the IR Diff view shows (with the changed
//! characters inside lines the hunk pairs up), and its blame gives every
//! line of snapshot `i` the step that last changed it, carried forward
//! through the same hunks — so diff and blame cannot disagree. The result
//! is saved under the user's cache directory, keyed by the trace file's
//! path, size and modification time, and read from there next time.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use pliron_inspect_protocol::trace::CrabbitTraceFile;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use similar::DiffOp;

use crate::line_diff::diff_lines;
use tokio::sync::OnceCell;

/// Bumped whenever [Step] or the way it is computed changes, so saved
/// results of an older computation are not reused.
const FORMAT: u32 = 5;

/// One step of a trace: snapshot `i` against snapshot `i - 1`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Step {
    /// The snapshot is the previous one, byte for byte.
    pub identical: bool,
    /// The changed regions, in order.
    pub hunks: Vec<Hunk>,
    /// Who last changed each line of this snapshot, as runs of
    /// `[step, line count]` covering its lines in order; step 0 is the
    /// initial snapshot.
    pub blame: Vec<[u32; 2]>,
}

/// A changed region: lines `[start, end)` (1-based) of the previous
/// snapshot replaced by lines `[start, end)` of this one; an empty range
/// is an insertion or deletion before its start line.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Hunk {
    pub original: [u32; 2],
    pub modified: [u32; 2],
    /// The lines of the hunk that are the same op (or block) on both sides,
    /// `[original line, modified line]`, increasing on both sides; see
    /// [identity].
    pub pairs: Vec<[u32; 2]>,
    /// The changed characters of each pair: `[original line, start column,
    /// end column, modified line, start column, end column]`, 1-based,
    /// columns in UTF-16 units and end-exclusive, as the editor counts them.
    pub inner: Vec<[u32; 6]>,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    format: u32,
    steps: Vec<Step>,
}

/// The steps of one trace, once they are computed or read.
type Steps = Arc<OnceCell<Arc<Vec<Step>>>>;

/// The steps of every trace asked for, computed (or read) once per trace.
#[derive(Default)]
pub struct Histories {
    traces: Mutex<HashMap<PathBuf, Steps>>,
}

impl Histories {
    /// The steps of the trace at `path` (canonical): from memory, else from
    /// the saved result, else computed and saved.
    pub async fn steps(&self, path: &Path) -> Result<Arc<Vec<Step>>> {
        let cell = self
            .traces
            .lock()
            .unwrap()
            .entry(path.to_path_buf())
            .or_default()
            .clone();
        let path = path.to_path_buf();
        cell.get_or_try_init(|| async move {
            tokio::task::spawn_blocking(move || load_or_compute(&path))
                .await
                .context("the history computation panicked")?
                .map(Arc::new)
        })
        .await
        .cloned()
    }
}

fn load_or_compute(path: &Path) -> Result<Vec<Step>> {
    let saved_at = saved_path(path)?;
    if let Some(steps) = std::fs::read(&saved_at)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Saved>(&bytes).ok())
        .filter(|saved| saved.format == FORMAT)
        .map(|saved| saved.steps)
    {
        return Ok(steps);
    }
    let trace = CrabbitTraceFile::read(path)?;
    let texts: Vec<&str> = trace.ir_dumps.iter().map(|dump| dump.ir.as_str()).collect();
    let steps = compute(&texts);
    // Saving is a convenience: a failure only means computing again.
    let saved = Saved {
        format: FORMAT,
        steps,
    };
    if let Err(error) = save(&saved_at, &saved) {
        eprintln!(
            "pliron-inspect: could not save the history of {}: {error:#}",
            path.display()
        );
    }
    Ok(saved.steps)
}

/// Where the steps of the trace at `path` are saved: named after its path,
/// size and modification time, so a rewritten trace is computed again.
fn saved_path(path: &Path) -> Result<PathBuf> {
    let meta = std::fs::metadata(path).with_context(|| path.display().to_string())?;
    let modified = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_nanos());
    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    hasher.update(meta.len().to_le_bytes());
    hasher.update(modified.to_le_bytes());
    let key: String = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let dir = dirs::cache_dir()
        .context("no cache directory for this user")?
        .join("pliron-inspect")
        .join("history");
    Ok(dir.join(format!("{key}.json")))
}

fn save(at: &Path, saved: &Saved) -> Result<()> {
    std::fs::create_dir_all(at.parent().expect("a file under the cache directory"))?;
    let partial = at.with_extension("json.partial");
    std::fs::write(&partial, serde_json::to_vec(saved)?)?;
    std::fs::rename(&partial, at)?;
    Ok(())
}

/// The steps of a trace whose snapshots print as `texts`, in order; step 0
/// (the initial snapshot) has no hunks.
pub fn compute(texts: &[&str]) -> Vec<Step> {
    let lines: Vec<Vec<&str>> = texts
        .iter()
        .map(|text| text.split('\n').collect())
        .collect();
    // The diffs are independent of each other: compute them side by side.
    let mut ops: Vec<Vec<DiffOp>> = vec![Vec::new(); texts.len()];
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get());
    let chunk = texts.len().div_ceil(workers).max(1);
    std::thread::scope(|scope| {
        for (offset, out) in ops.chunks_mut(chunk).enumerate() {
            let lines = &lines;
            let texts = &texts;
            scope.spawn(move || {
                for (k, slot) in out.iter_mut().enumerate() {
                    let i = offset * chunk + k;
                    if i > 0 && texts[i] != texts[i - 1] {
                        *slot = diff_lines(&lines[i - 1], &lines[i]);
                    }
                }
            });
        }
    });

    let mut steps = Vec::with_capacity(texts.len());
    let mut blame: Vec<u32> = Vec::new();
    for (i, current) in lines.iter().enumerate() {
        if i == 0 {
            blame = vec![0; current.len()];
            steps.push(Step {
                identical: false,
                hunks: Vec::new(),
                blame: runs(&blame),
            });
            continue;
        }
        if texts[i] == texts[i - 1] {
            steps.push(Step {
                identical: true,
                hunks: Vec::new(),
                blame: runs(&blame),
            });
            continue;
        }
        let mut next = vec![i as u32; current.len()];
        for op in &ops[i] {
            if let DiffOp::Equal {
                old_index,
                new_index,
                len,
            } = *op
            {
                next[new_index..new_index + len]
                    .copy_from_slice(&blame[old_index..old_index + len]);
            }
        }
        blame = next;
        steps.push(Step {
            identical: false,
            hunks: hunks(&ops[i], &lines[i - 1], current),
            blame: runs(&blame),
        });
    }
    steps
}

/// Consecutive non-equal ops, merged into hunks.
fn hunks(ops: &[DiffOp], old: &[&str], new: &[&str]) -> Vec<Hunk> {
    let mut hunks = Vec::new();
    let mut open: Option<(usize, usize, usize, usize)> = None;
    let mut close = |open: &mut Option<(usize, usize, usize, usize)>| {
        if let Some((old_start, old_end, new_start, new_end)) = open.take() {
            let pairs = pair_lines(old, new, old_start..old_end, new_start..new_end);
            hunks.push(Hunk {
                original: [old_start as u32 + 1, old_end as u32 + 1],
                modified: [new_start as u32 + 1, new_end as u32 + 1],
                inner: inner_changes(old, new, &pairs),
                pairs: pairs
                    .iter()
                    .map(|&(o, n)| [o as u32 + 1, n as u32 + 1])
                    .collect(),
            });
        }
    };
    for op in ops {
        let (old_range, new_range) = (op.old_range(), op.new_range());
        if matches!(op, DiffOp::Equal { .. }) {
            close(&mut open);
            continue;
        }
        open = Some(match open {
            Some((old_start, _, new_start, _)) => {
                (old_start, old_range.end, new_start, new_range.end)
            }
            None => (
                old_range.start,
                old_range.end,
                new_range.start,
                new_range.end,
            ),
        });
    }
    close(&mut open);
    hunks
}

/// What a printed line is the same thing as on the other side of a change:
/// the op defining the same values (`v1, v2 = ...`), the block with the
/// same label (`^bb1(...)`), else the same text once its location marker
/// (` !N`, renumbered by every print) is set aside.
fn identity(line: &str) -> String {
    let text = line.trim();
    if let Some((results, _)) = text.split_once(" = ")
        && !results.is_empty()
        && results.split(", ").all(is_value_name)
    {
        return format!("values {results}");
    }
    if let Some(label) = text.strip_prefix('^') {
        let end = label.find(['(', ' ', ':']).unwrap_or(label.len());
        return format!("block {}", &label[..end]);
    }
    format!("text {}", without_marker(text))
}

fn is_value_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `text` without a trailing location marker: ` !12` or ` !12;`.
fn without_marker(text: &str) -> String {
    let (body, end) = match text.strip_suffix(';') {
        Some(body) => (body, ";"),
        None => (text, ""),
    };
    match body.rsplit_once(" !") {
        Some((head, digits))
            if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) =>
        {
            format!("{}{end}", head.trim_end())
        }
        _ => text.to_string(),
    }
}

/// The lines of a hunk that are the same thing on both sides: the lines a
/// diff of the two sides' [identity] keeps, so repeated lines pair in
/// order too. Increasing on both sides.
fn pair_lines(
    old: &[&str],
    new: &[&str],
    old_lines: std::ops::Range<usize>,
    new_lines: std::ops::Range<usize>,
) -> Vec<(usize, usize)> {
    let old_keys: Vec<String> = old[old_lines.clone()]
        .iter()
        .map(|line| identity(line))
        .collect();
    let new_keys: Vec<String> = new[new_lines.clone()]
        .iter()
        .map(|line| identity(line))
        .collect();
    let old_keys: Vec<&str> = old_keys.iter().map(String::as_str).collect();
    let new_keys: Vec<&str> = new_keys.iter().map(String::as_str).collect();
    let mut pairs = Vec::new();
    for op in diff_lines(&old_keys, &new_keys) {
        if let DiffOp::Equal {
            old_index,
            new_index,
            len,
        } = op
        {
            pairs.extend((0..len).map(|k| {
                (
                    old_lines.start + old_index + k,
                    new_lines.start + new_index + k,
                )
            }));
        }
    }
    pairs
}

/// The changed characters of each pair: everything between the characters
/// the two lines start and end with in common.
fn inner_changes(old: &[&str], new: &[&str], pairs: &[(usize, usize)]) -> Vec<[u32; 6]> {
    let mut inner = Vec::new();
    for &(o, n) in pairs {
        let a: Vec<char> = old[o].chars().collect();
        let b: Vec<char> = new[n].chars().collect();
        let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
        let suffix = a[prefix..]
            .iter()
            .rev()
            .zip(b[prefix..].iter().rev())
            .take_while(|(x, y)| x == y)
            .count();
        let (a_cols, b_cols) = (utf16_columns(&a), utf16_columns(&b));
        inner.push([
            o as u32 + 1,
            a_cols[prefix],
            a_cols[a.len() - suffix],
            n as u32 + 1,
            b_cols[prefix],
            b_cols[b.len() - suffix],
        ]);
    }
    inner
}

/// The editor column (1-based, UTF-16 units) of each char index of a line,
/// and of its end.
fn utf16_columns(chars: &[char]) -> Vec<u32> {
    let mut columns = Vec::with_capacity(chars.len() + 1);
    let mut column = 1u32;
    for c in chars {
        columns.push(column);
        column += c.len_utf16() as u32;
    }
    columns.push(column);
    columns
}

/// `[value, count]` runs of `values`.
fn runs(values: &[u32]) -> Vec<[u32; 2]> {
    let mut runs: Vec<[u32; 2]> = Vec::new();
    for &value in values {
        match runs.last_mut() {
            Some(run) if run[0] == value => run[1] += 1,
            _ => runs.push([value, 1]),
        }
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expand(runs: &[[u32; 2]]) -> Vec<u32> {
        runs.iter()
            .flat_map(|&[value, count]| std::iter::repeat_n(value, count as usize))
            .collect()
    }

    #[test]
    fn blame_follows_the_hunks() {
        let steps = compute(&["a\nb\nc", "a\nb\nc", "a\nB\nc\nd", "x\nB\nc\nd"]);
        assert!(steps[1].identical && steps[1].hunks.is_empty());
        assert_eq!(expand(&steps[1].blame), [0, 0, 0]);
        // b → B and the new d: step 2; a and c keep the initial blame.
        assert_eq!(expand(&steps[2].blame), [0, 2, 0, 2]);
        assert_eq!(
            steps[2].hunks,
            [
                Hunk {
                    original: [2, 3],
                    modified: [2, 3],
                    pairs: vec![],
                    inner: vec![],
                },
                Hunk {
                    original: [4, 4],
                    modified: [4, 5],
                    pairs: vec![],
                    inner: vec![],
                },
            ]
        );
        assert_eq!(expand(&steps[3].blame), [3, 2, 0, 2]);
    }

    #[test]
    fn inner_changes_count_utf16_columns() {
        let steps = compute(&["é !12", "é !13"]);
        // The same text but for the marker: a pair. `é` is one UTF-16
        // unit; the changed digit is column 5.
        assert_eq!(steps[1].hunks[0].pairs, vec![[1, 1]]);
        assert_eq!(steps[1].hunks[0].inner, vec![[1, 5, 6, 1, 5, 6]]);
    }

    #[test]
    fn repeated_lines_pair_in_order() {
        let old = "  str x1 !1\n  str x1 !2\n  ret !3";
        let new = "  mov x0 !4\n  str x1 !5\n  str x1 !6\n  ret !7";
        let steps = compute(&[old, new]);
        // The two stores and the ret pair up; the new mov has no counterpart.
        assert_eq!(steps[1].hunks[0].pairs, vec![[1, 2], [2, 3], [3, 4]]);
    }

    #[test]
    fn lines_pair_by_what_they_define_not_by_position() {
        let old = "  llvm.br ^bb1 !1\n\n^bb1():\n  v1 = a !2\n  v2 = b !3";
        let new = "^bb0():\n  v1 = a !5\n  v2 = b !6";
        let steps = compute(&[old, new]);
        let pairs = &steps[1].hunks[0].pairs;
        // v1 and v2 pair with themselves; ^bb1 has no counterpart.
        assert_eq!(pairs, &vec![[4, 2], [5, 3]]);
    }
}
