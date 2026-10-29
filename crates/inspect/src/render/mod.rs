//! CFG and tree render documents, built from the IR model that the
//! project's dialect engine reports for a document (pliron-lsp's
//! `pliron/model`). The project's own parsers produce the model, so every
//! dialect renders without any code in the project.

use std::collections::BTreeMap;

use pliron_inspect_protocol::types::{Diagnostic, DiagnosticSeverity, RenderDocument};
use pliron_lsp_protocol::Model;
use serde::Deserialize;

pub mod cfg;
pub mod flame;
pub mod timeline;
pub mod tree;

pub const RENDER_PROTOCOL_VERSION: u32 = 1;

/// An LSP position: 0-based line, UTF-16 column.
#[derive(Clone, Copy, Debug, Deserialize)]
pub struct LspPosition {
    pub line: u32,
    pub character: u32,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct LspRange {
    pub start: LspPosition,
    pub end: LspPosition,
}

/// pliron-lsp's answer to `pliron/model`: no model while the engine has not
/// analyzed the document's current text.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelResponse {
    pub model: Option<Model>,
    /// The whole source range of each op of the model, by op index.
    #[serde(default)]
    pub op_ranges: Vec<Option<LspRange>>,
    #[serde(default)]
    pub note: Option<String>,
}

/// A parsed document: its text and the engine's model of it.
pub struct Ir {
    text: std::sync::Arc<String>,
    line_starts: Vec<usize>,
    model: Model,
    op_ranges: Vec<Option<LspRange>>,
}

impl Ir {
    pub fn new(text: std::sync::Arc<String>, model: Model, op_ranges: Vec<Option<LspRange>>) -> Ir {
        let line_starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        Ir {
            text,
            line_starts,
            model,
            op_ranges,
        }
    }

    pub fn model(&self) -> &Model {
        &self.model
    }

    fn offset(&self, p: LspPosition) -> usize {
        let Some(&start) = self.line_starts.get(p.line as usize) else {
            return self.text.len();
        };
        let mut units = 0;
        for (i, ch) in self.text[start..].char_indices() {
            if units >= p.character || ch == '\n' {
                return start + i;
            }
            units += ch.len_utf16() as u32;
        }
        self.text.len()
    }

    /// The op's source text.
    fn op_text(&self, op: u32) -> &str {
        match self.op_ranges.get(op as usize).copied().flatten() {
            Some(r) => {
                let (s, e) = (self.offset(r.start), self.offset(r.end));
                &self.text[s.min(e)..e]
            }
            None => self.opid(op),
        }
    }

    /// The op's source lines, trimmed, without blank ones.
    fn op_lines(&self, op: u32) -> Vec<String> {
        self.op_text(op)
            .lines()
            .map(|line| line.trim().to_string())
            .filter(|line| !line.is_empty())
            .collect()
    }

    /// The 1-based line the op starts on in the snapshot.
    fn op_line(&self, op: u32) -> Option<u32> {
        self.op_ranges
            .get(op as usize)
            .copied()
            .flatten()
            .map(|r| r.start.line + 1)
    }

    fn op_first_line(&self, op: u32) -> String {
        self.op_lines(op).into_iter().next().unwrap_or_default()
    }

    fn opid(&self, op: u32) -> &str {
        &self.model.ops[op as usize].opid
    }

    fn block_label(&self, block: u32) -> String {
        match &self.model.blocks[block as usize].label {
            Some(label) => format!("^{label}"),
            None => format!("^bb{block}"),
        }
    }

    fn value_name(&self, value: u32) -> String {
        let v = &self.model.values[value as usize];
        match &v.given_name {
            Some(name) => name.clone(),
            None => format!("v{value}"),
        }
    }

    fn region_has_blocks(&self, region: u32) -> bool {
        !self.model.regions[region as usize].blocks.is_empty()
    }

    fn op_has_blocks(&self, op: u32) -> bool {
        self.model.ops[op as usize]
            .regions
            .iter()
            .any(|r| self.region_has_blocks(*r))
    }
}

pub fn diagnostic_document(
    view: impl Into<String>,
    snapshot_id: impl Into<String>,
    title: impl Into<String>,
    code: impl Into<String>,
    message: impl Into<String>,
) -> RenderDocument {
    RenderDocument {
        version: RENDER_PROTOCOL_VERSION,
        view: view.into(),
        title: Some(title.into()),
        snapshot_id: snapshot_id.into(),
        entities: Vec::new(),
        graph: None,
        tree: None,
        diagnostics: vec![Diagnostic {
            severity: DiagnosticSeverity::Info,
            code: code.into(),
            message: message.into(),
            entity_id: None,
            attrs: BTreeMap::new(),
        }],
    }
}

fn op_entity_id(op: u32) -> String {
    format!("op:{op}")
}

fn block_entity_id(block: u32) -> String {
    format!("block:{block}")
}

fn region_entity_id(region: u32) -> String {
    format!("region:{region}")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use pliron_lsp_protocol::{BlockInfo, OpInfo, RegionInfo, ValueDef, ValueInfo, op_traits};

    /// A module holding `@f` with a diamond CFG, as an engine reports it.
    pub(crate) const TEXT: &str = "builtin.module @m {
  ^entry():
  llvm.func @f: llvm.func <() -> ()> {
    ^entry(x: builtin.integer i1):
    llvm.cond_br if x ^a() else ^b()

    ^a():
    llvm.br ^exit()

    ^b():
    llvm.br ^exit()

    ^exit():
    llvm.return
  }
}
";

    fn range(text: &str, needle: &str, len: usize) -> Option<LspRange> {
        let start = text.find(needle).unwrap();
        let pos = |off: usize| {
            let line = text[..off].matches('\n').count() as u32;
            let col = off - text[..off].rfind('\n').map(|p| p + 1).unwrap_or(0);
            LspPosition {
                line,
                character: col as u32,
            }
        };
        Some(LspRange {
            start: pos(start),
            end: pos(start + len),
        })
    }

    pub(crate) fn diamond() -> Ir {
        let op = |opid: &str, parent_block: Option<u32>| OpInfo {
            opid: opid.into(),
            parent_block,
            ..Default::default()
        };
        let mut module = op("builtin.module", None);
        module.regions = vec![0];
        module.symbol = Some("m".into());
        module.traits = op_traits::SYMBOL | op_traits::SYMBOL_TABLE;
        let mut func = op("llvm.func", Some(0));
        func.regions = vec![1];
        func.symbol = Some("f".into());
        func.traits = op_traits::SYMBOL;
        let mut cond_br = op("llvm.cond_br", Some(1));
        cond_br.operands = vec![0];
        cond_br.successors = vec![2, 3];
        let mut br_a = op("llvm.br", Some(2));
        br_a.successors = vec![4];
        let mut br_b = op("llvm.br", Some(3));
        br_b.successors = vec![4];
        let ret = op("llvm.return", Some(4));
        let block = |label: &str, region: u32, args: Vec<u32>, ops: Vec<u32>| BlockInfo {
            label: Some(label.into()),
            pos: None,
            region,
            args,
            ops,
        };
        let model = Model {
            root: 0,
            ops: vec![module, func, cond_br, br_a, br_b, ret],
            regions: vec![
                RegionInfo {
                    parent_op: 0,
                    blocks: vec![0],
                },
                RegionInfo {
                    parent_op: 1,
                    blocks: vec![1, 2, 3, 4],
                },
            ],
            blocks: vec![
                block("entry", 0, vec![], vec![1]),
                block("entry", 1, vec![0], vec![2]),
                block("a", 1, vec![], vec![3]),
                block("b", 1, vec![], vec![4]),
                block("exit", 1, vec![], vec![5]),
            ],
            values: vec![ValueInfo {
                def: ValueDef::Arg { block: 1, index: 0 },
                ty: "builtin.integer i1".into(),
                given_name: Some("x".into()),
            }],
        };
        let module_len = TEXT.trim_end().len();
        let func_start = TEXT.find("llvm.func").unwrap();
        let func_len = TEXT[func_start..].find("\n  }").unwrap() + 4;
        let op_ranges = vec![
            range(TEXT, "builtin.module", module_len),
            range(TEXT, "llvm.func", func_len),
            range(
                TEXT,
                "llvm.cond_br",
                "llvm.cond_br if x ^a() else ^b()".len(),
            ),
            range(TEXT, "llvm.br ^exit", "llvm.br ^exit()".len()),
            // The second `llvm.br`, the one in `^b`.
            range(
                TEXT,
                "llvm.br ^exit()\n\n    ^exit",
                "llvm.br ^exit()".len(),
            ),
            range(TEXT, "llvm.return", "llvm.return".len()),
        ];
        Ir::new(std::sync::Arc::new(TEXT.to_string()), model, op_ranges)
    }

    #[test]
    fn op_text_comes_from_the_source() {
        let ir = diamond();
        assert_eq!(ir.op_text(2), "llvm.cond_br if x ^a() else ^b()");
        assert_eq!(ir.op_first_line(1), "llvm.func @f: llvm.func <() -> ()> {");
        assert_eq!(ir.op_lines(5), ["llvm.return"]);
        assert_eq!(ir.block_label(4), "^exit");
        assert_eq!(ir.value_name(0), "x");
    }
}
