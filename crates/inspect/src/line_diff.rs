//! A line diff fit for printed IR.
//!
//! Printed IR is mostly lines that occur once (each names its own values,
//! blocks, location markers), so a patience diff aligns it cheaply: lines
//! that occur exactly once on each side anchor the alignment, and only the
//! regions between anchors are diffed again, recursively. A region with no
//! line in common is a plain replacement; Myers only runs on small regions
//! that share lines but have none unique (`}`, repeated branches). A
//! generic Myers diff of two snapshots, by contrast, costs time in
//! proportion to the lines that changed, and renumbered markers change
//! thousands of them.

use std::collections::HashMap;
use std::ops::Range;

use similar::{Algorithm, DiffOp, capture_diff_slices};

/// A region without anchors is diffed by Myers only up to this many line
/// pairs (old × new); a larger one is one replacement.
const MYERS_LIMIT: usize = 1 << 20;

/// The diff of `old` against `new`, as ops in order.
pub fn diff_lines<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<DiffOp> {
    // Lines as ids, so comparing two is comparing two integers.
    let mut ids: HashMap<&'a str, u32> = HashMap::new();
    let mut intern = |line: &'a str| -> u32 {
        let next = ids.len() as u32;
        *ids.entry(line).or_insert(next)
    };
    let a: Vec<u32> = old.iter().map(|&line| intern(line)).collect();
    let b: Vec<u32> = new.iter().map(|&line| intern(line)).collect();
    let mut pieces = Vec::new();
    diff_region(&a, 0..a.len(), &b, 0..b.len(), &mut pieces);
    to_ops(pieces)
}

/// Lines `old` of `a` correspond to lines `new` of `b`: equal when
/// `same`, else replaced.
struct Piece {
    old: Range<usize>,
    new: Range<usize>,
    same: bool,
}

fn diff_region(a: &[u32], old: Range<usize>, b: &[u32], new: Range<usize>, out: &mut Vec<Piece>) {
    // Lines the two regions start and end with in common.
    let prefix = a[old.clone()]
        .iter()
        .zip(&b[new.clone()])
        .take_while(|(x, y)| x == y)
        .count();
    let suffix = a[old.start + prefix..old.end]
        .iter()
        .rev()
        .zip(b[new.start + prefix..new.end].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    if prefix > 0 {
        out.push(Piece {
            old: old.start..old.start + prefix,
            new: new.start..new.start + prefix,
            same: true,
        });
    }
    let old_mid = old.start + prefix..old.end - suffix;
    let new_mid = new.start + prefix..new.end - suffix;
    diff_middle(a, old_mid, b, new_mid, out);
    if suffix > 0 {
        out.push(Piece {
            old: old.end - suffix..old.end,
            new: new.end - suffix..new.end,
            same: true,
        });
    }
}

fn diff_middle(a: &[u32], old: Range<usize>, b: &[u32], new: Range<usize>, out: &mut Vec<Piece>) {
    if old.is_empty() && new.is_empty() {
        return;
    }
    if old.is_empty() || new.is_empty() {
        out.push(Piece {
            old,
            new,
            same: false,
        });
        return;
    }
    let anchors = anchors(a, old.clone(), b, new.clone());
    if !anchors.is_empty() {
        let (mut old_at, mut new_at) = (old.start, new.start);
        for (i, j) in anchors {
            diff_region(a, old_at..i, b, new_at..j, out);
            out.push(Piece {
                old: i..i + 1,
                new: j..j + 1,
                same: true,
            });
            (old_at, new_at) = (i + 1, j + 1);
        }
        diff_region(a, old_at..old.end, b, new_at..new.end, out);
        return;
    }
    let shares_a_line = {
        let in_old: std::collections::HashSet<u32> = a[old.clone()].iter().copied().collect();
        b[new.clone()].iter().any(|id| in_old.contains(id))
    };
    if !shares_a_line || old.len() * new.len() > MYERS_LIMIT {
        out.push(Piece {
            old,
            new,
            same: false,
        });
        return;
    }
    for op in capture_diff_slices(Algorithm::Myers, &a[old.clone()], &b[new.clone()]) {
        let (o, n) = (op.old_range(), op.new_range());
        out.push(Piece {
            old: old.start + o.start..old.start + o.end,
            new: new.start + n.start..new.start + n.end,
            same: matches!(op, DiffOp::Equal { .. }),
        });
    }
}

/// The lines that occur exactly once in each region, as `(old, new)` index
/// pairs, reduced to the longest run increasing on both sides.
fn anchors(a: &[u32], old: Range<usize>, b: &[u32], new: Range<usize>) -> Vec<(usize, usize)> {
    // id → (count in old, index in old, count in new, index in new)
    let mut seen: HashMap<u32, (u32, usize, u32, usize)> = HashMap::new();
    for i in old {
        let entry = seen.entry(a[i]).or_insert((0, i, 0, 0));
        entry.0 += 1;
    }
    for j in new {
        if let Some(entry) = seen.get_mut(&b[j]) {
            entry.2 += 1;
            entry.3 = j;
        }
    }
    let mut pairs: Vec<(usize, usize)> = seen
        .into_values()
        .filter(|&(old_count, _, new_count, _)| old_count == 1 && new_count == 1)
        .map(|(_, i, _, j)| (i, j))
        .collect();
    pairs.sort_unstable();
    longest_increasing(&pairs)
}

/// The longest subsequence of `pairs` (sorted by old index) whose new
/// indices increase: patience sorting.
fn longest_increasing(pairs: &[(usize, usize)]) -> Vec<(usize, usize)> {
    // tails[k]: index into `pairs` of the smallest tail of a run of k + 1.
    let mut tails: Vec<usize> = Vec::new();
    let mut previous: Vec<Option<usize>> = vec![None; pairs.len()];
    for (index, &(_, j)) in pairs.iter().enumerate() {
        let k = tails.partition_point(|&t| pairs[t].1 < j);
        previous[index] = k.checked_sub(1).map(|k| tails[k]);
        if k == tails.len() {
            tails.push(index);
        } else {
            tails[k] = index;
        }
    }
    let mut run = Vec::with_capacity(tails.len());
    let mut at = tails.last().copied();
    while let Some(index) = at {
        run.push(pairs[index]);
        at = previous[index];
    }
    run.reverse();
    run
}

/// Pieces as ops: neighbouring pieces of the same kind merged, a change
/// with lines on both sides a replacement.
fn to_ops(pieces: Vec<Piece>) -> Vec<DiffOp> {
    let mut merged: Vec<Piece> = Vec::new();
    for piece in pieces {
        if piece.old.is_empty() && piece.new.is_empty() {
            continue;
        }
        match merged.last_mut() {
            Some(last) if last.same == piece.same => {
                last.old.end = piece.old.end;
                last.new.end = piece.new.end;
            }
            _ => merged.push(piece),
        }
    }
    merged
        .into_iter()
        .map(|piece| {
            let (old, new) = (piece.old, piece.new);
            if piece.same {
                DiffOp::Equal {
                    old_index: old.start,
                    new_index: new.start,
                    len: old.len(),
                }
            } else if old.is_empty() {
                DiffOp::Insert {
                    old_index: old.start,
                    new_index: new.start,
                    new_len: new.len(),
                }
            } else if new.is_empty() {
                DiffOp::Delete {
                    old_index: old.start,
                    old_len: old.len(),
                    new_index: new.start,
                }
            } else {
                DiffOp::Replace {
                    old_index: old.start,
                    old_len: old.len(),
                    new_index: new.start,
                    new_len: new.len(),
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ops rebuild `new` from `old`, and their equal lines are equal.
    fn check(old: &[&str], new: &[&str]) -> Vec<DiffOp> {
        let ops = diff_lines(old, new);
        let mut rebuilt: Vec<&str> = Vec::new();
        let (mut old_at, mut new_at) = (0, 0);
        for op in &ops {
            let (o, n) = (op.old_range(), op.new_range());
            assert_eq!((o.start, n.start), (old_at, new_at), "ops are contiguous");
            if matches!(op, DiffOp::Equal { .. }) {
                assert_eq!(&old[o.clone()], &new[n.clone()]);
            }
            rebuilt.extend_from_slice(&new[n.clone()]);
            (old_at, new_at) = (o.end, n.end);
        }
        assert_eq!((old_at, new_at), (old.len(), new.len()));
        assert_eq!(rebuilt, new);
        ops
    }

    #[test]
    fn unique_lines_anchor_around_a_renumbered_stretch() {
        let old = ["a", "x !1", "y !2", "b", "}", "}"];
        let new = ["a", "n", "x !2", "y !3", "b", "}", "}"];
        let ops = check(&old, &new);
        let changed: usize = ops
            .iter()
            .filter(|op| !matches!(op, DiffOp::Equal { .. }))
            .map(|op| op.new_range().len())
            .sum();
        assert_eq!(changed, 3, "{ops:?}");
    }

    #[test]
    fn edge_cases() {
        check(&[], &[]);
        check(&["a"], &[]);
        check(&[], &["a"]);
        check(&["}", "}", "x"], &["x", "}", "}"]);
        check(&["a", "b", "c"], &["c", "b", "a"]);
    }
}
