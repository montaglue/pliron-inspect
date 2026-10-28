//! Language queries over an immutable IR document, for the resident
//! server's `ir_hover` / `ir_definition` / `ir_references` /
//! `ir_diagnostics` commands.
//!
//! Two layers, as in pliron-lsp. The dialect-agnostic syntax layer
//! (`pliron-ir-syntax`) gives every name its definition and its uses with
//! pliron's own scoping rules, instantly and whether or not the text
//! parses. Parsing the text with the dialects linked into this binary adds
//! what only the real parsers know: exact types, op signatures, and the
//! parser's and verifier's errors. pliron's IR keeps a start location per
//! operation and block but no spans for value uses, so the parsed facts
//! are joined to the syntax layer by (name, definition line).
//!
//! Positions are the wire's [`DocPosition`]: 0-based line, UTF-16 column.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};

use pliron::builtin::attributes::{ATTR_KEY_GIVEN_NAMES, IdentifierAttr};
use pliron::builtin::op_interfaces::ATTR_KEY_SYM_NAME;
use pliron::common_traits::Named;
use pliron::context::{Context, Ptr};
use pliron::identifier::Identifier;
use pliron::irfmt::parsers::spaced;
use pliron::linked_list::ContainsLinkedList;
use pliron::location::Location;
use pliron::operation::{Operation, verify_operation};
use pliron::parsable::parse_from_str;
use pliron::printable::Printable;
use pliron::r#type::Typed;
use pliron_inspect_protocol::server::{DocPosition, DocRange};
use pliron_ir_syntax::{
    Analysis, DefKind, Encoding, Knowledge, LineIndex, Offset, Role, Severity as SyntaxSeverity,
    analyze,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// A diagnostic of an IR document.
#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub range: DocRange,
    pub severity: Severity,
    /// `pliron-parse`, `pliron-verify` or `syntax`.
    pub source: &'static str,
    pub message: String,
}

/// What the parsed module knows about a value defined on a line.
struct ValueFact {
    ty: String,
    /// "result of `dialect.op`" or "block argument of `^bb`".
    origin: String,
}

/// Facts from the parsed module, keyed by what the syntax layer can see:
/// names and the 0-based line of their definition.
#[derive(Default)]
struct Facts {
    values: HashMap<(String, u32), ValueFact>,
    /// (label, line) -> rendered block signature.
    blocks: HashMap<(String, u32), String>,
    /// (line, op id) -> hover markdown.
    ops: HashMap<(u32, String), String>,
}

/// The analysis of one immutable IR document.
pub struct Document {
    text: String,
    line_index: LineIndex,
    syntax: Analysis,
    facts: Facts,
    /// Did the text parse into a module with the linked dialects?
    pub parsed: bool,
    pub diagnostics: Vec<Diagnostic>,
}

impl Document {
    /// Analyze `text` with the dialects registered in `ctx` (a fresh
    /// context from the driver's hooks). Parser and verifier panics are
    /// reported as diagnostics.
    pub fn analyze(text: String, ctx: &mut Context) -> Document {
        let line_index = LineIndex::new(&text);
        let syntax = analyze(&text, &Knowledge::default());
        let mut diagnostics = Vec::new();
        let mut facts = Facts::default();
        let mut parsed = false;

        let parse_result = catch_unwind(AssertUnwindSafe(|| {
            parse_from_str(spaced(Operation::top_level_parser()), ctx, text.as_str())
        }));
        match parse_result {
            Ok(Ok(op)) => {
                parsed = true;
                match catch_unwind(AssertUnwindSafe(|| verify_operation(op, ctx))) {
                    Ok(Ok(())) => {}
                    Ok(Err(err)) => diagnostics.push(Diagnostic {
                        range: span_of_location(&err.loc, &line_index, &text),
                        severity: Severity::Error,
                        source: "pliron-verify",
                        message: strip_location_prefix(&err.disp(ctx).to_string()),
                    }),
                    Err(panic) => diagnostics.push(Diagnostic {
                        range: first_line(&line_index, &text),
                        severity: Severity::Error,
                        source: "pliron-verify",
                        message: format!("verifier panicked: {}", panic_message(panic)),
                    }),
                }
                collect_op(ctx, op, &mut facts);
            }
            Ok(Err(err)) => diagnostics.push(Diagnostic {
                range: span_of_location(&err.loc, &line_index, &text),
                severity: Severity::Error,
                source: "pliron-parse",
                message: strip_location_prefix(&err.disp(ctx).to_string()),
            }),
            Err(panic) => diagnostics.push(Diagnostic {
                range: first_line(&line_index, &text),
                severity: Severity::Error,
                source: "pliron-parse",
                message: format!("parser panicked: {}", panic_message(panic)),
            }),
        }

        // The syntax layer's findings (unbalanced brackets, a missing `;`,
        // undefined names) locate problems precisely; the real parser
        // stops at the first one. Custom op syntax can look like a
        // misuse to the syntax layer, so they only count when the text
        // does not parse.
        if !parsed {
            for d in &syntax.diagnostics {
                diagnostics.push(Diagnostic {
                    range: range_of(&line_index, &text, d.start, d.end),
                    severity: match d.severity {
                        SyntaxSeverity::Error => Severity::Error,
                        SyntaxSeverity::Warning => Severity::Warning,
                        SyntaxSeverity::Info => Severity::Info,
                    },
                    source: "syntax",
                    message: d.message.clone(),
                });
            }
        }

        Document {
            text,
            line_index,
            syntax,
            facts,
            parsed,
            diagnostics,
        }
    }

    fn offset(&self, pos: DocPosition) -> Offset {
        self.line_index.offset(
            pliron_ir_syntax::LinePos {
                line: pos.line,
                col: pos.character,
            },
            &self.text,
            Encoding::Utf16,
        )
    }

    fn range(&self, start: Offset, end: Offset) -> DocRange {
        range_of(&self.line_index, &self.text, start, end)
    }

    fn token_range(&self, tok: pliron_ir_syntax::TokIdx) -> DocRange {
        let t = self.syntax.tree.tok(tok);
        self.range(t.start, t.end)
    }

    /// Definition site of the name at `pos`, if any.
    pub fn definition(&self, pos: DocPosition) -> Option<DocRange> {
        let tok = self.syntax.tree.token_at(self.offset(pos))?;
        let def = self.syntax.def_of_token(tok)?;
        Some(self.token_range(self.syntax.def(def).tok))
    }

    /// All references of the name at `pos`, optionally including its
    /// definition (first).
    pub fn references(&self, pos: DocPosition, include_declaration: bool) -> Vec<DocRange> {
        let Some(tok) = self.syntax.tree.token_at(self.offset(pos)) else {
            return vec![];
        };
        let Some(def) = self.syntax.def_of_token(tok) else {
            return vec![];
        };
        let mut out = Vec::new();
        if include_declaration {
            out.push(self.token_range(self.syntax.def(def).tok));
        }
        for u in &self.syntax.uses[def.0 as usize] {
            out.push(self.token_range(*u));
        }
        out
    }

    /// Hover for the token at `pos`: values, labels and symbols show their
    /// definition (with the exact type when the module parsed); op names
    /// show the op's signature.
    pub fn hover(&self, pos: DocPosition) -> Option<(DocRange, String)> {
        let tok = self.syntax.tree.token_at(self.offset(pos))?;
        let t = self.syntax.tree.tok(tok);
        if let Some(def) = self.syntax.def_of_token(tok) {
            let d = self.syntax.def(def);
            let def_tok = self.syntax.tree.tok(d.tok);
            let line = self.line_index.line_of(def_tok.start);
            let name = &d.name;
            let text = match d.kind {
                DefKind::Result | DefKind::BlockArg => {
                    match self.facts.values.get(&(name.clone(), line)) {
                        Some(f) => format!(
                            "`{name}: {}`\n\n{} (line {})",
                            f.ty.trim(),
                            f.origin,
                            line + 1
                        ),
                        None => match d.ty {
                            Some(ty) => format!(
                                "`{name}: {}`\n\nSSA value (line {})",
                                &self.text[ty.start as usize..ty.end as usize],
                                line + 1
                            ),
                            None => format!("`{name}` — SSA value (line {})", line + 1),
                        },
                    }
                }
                DefKind::Label => match self.facts.blocks.get(&(name.clone(), line)) {
                    Some(sig) => format!("`{sig}`\n\nblock (line {})", line + 1),
                    None => format!("`^{name}` — block (line {})", line + 1),
                },
                DefKind::Symbol => {
                    let opid = d
                        .stmt
                        .and_then(|s| self.syntax.tree.stmt(s).op_name)
                        .map(|t| self.syntax.tree.tok(t).text(&self.text))
                        .unwrap_or("op");
                    format!(
                        "`@{name}`\n\nsymbol defined by `{opid}` (line {})",
                        line + 1
                    )
                }
                DefKind::Outline => {
                    format!("`!{name}` — outlined attribute entry (line {})", line + 1)
                }
            };
            return Some((self.range(t.start, t.end), text));
        }
        if self.syntax.role(tok) == Role::OpName {
            let opid = t.text(&self.text);
            let line = self.line_index.line_of(t.start);
            let text = self
                .facts
                .ops
                .get(&(line, opid.to_string()))
                .cloned()
                .unwrap_or_else(|| format!("`{opid}`"));
            return Some((self.range(t.start, t.end), text));
        }
        None
    }
}

fn panic_message(panic: Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".to_string())
}

fn range_of(li: &LineIndex, text: &str, start: Offset, end: Offset) -> DocRange {
    let to_pos = |off: Offset| {
        let p = li.position(off, text, Encoding::Utf16);
        DocPosition {
            line: p.line,
            character: p.col,
        }
    };
    DocRange {
        start: to_pos(start),
        end: to_pos(end.max(start)),
    }
}

fn first_line(li: &LineIndex, text: &str) -> DocRange {
    range_of(li, text, 0, li.line_end(0, text))
}

/// pliron error messages rendered with `disp` start with the location,
/// as `[<in-memory>: line: N, column: M] ` or `<in-memory>: line: N,
/// column: M\n` depending on the version; the position is already carried
/// by the diagnostic range, so strip it.
fn strip_location_prefix(msg: &str) -> String {
    let msg = msg.trim();
    if let Some(rest) = msg.strip_prefix("[<in-memory>")
        && let Some(close) = rest.find(']')
    {
        return rest[close + 1..].trim().to_string();
    }
    if let Some(rest) = msg.strip_prefix("<in-memory>") {
        let rest = rest.trim_start_matches([':', ' ']);
        if let Some(pos) = rest.find('\n') {
            let head = &rest[..pos];
            if head.starts_with("line") {
                return rest[pos + 1..].trim().to_string();
            }
        }
    }
    msg.to_string()
}

/// Best-effort range for a pliron [`Location`]: from its position to the
/// end of that line (or the first line when it has no position).
fn span_of_location(loc: &Location, li: &LineIndex, text: &str) -> DocRange {
    match src_pos_of(loc) {
        Some((line, col)) => {
            let start = li.offset_of_pliron(line.max(1) as u32, col.max(1) as u32, text);
            let end = li.line_end(li.line_of(start), text);
            range_of(li, text, start, end.max(start + 1).min(li.len()))
        }
        None => first_line(li, text),
    }
}

fn src_pos_of(loc: &Location) -> Option<(i32, i32)> {
    match loc {
        Location::SrcPos { pos, .. } => Some((pos.line, pos.column)),
        Location::Fused { locations, .. } => locations.iter().find_map(src_pos_of),
        Location::Named { child_loc, .. } => src_pos_of(child_loc),
        Location::CallSite { callee, caller } => src_pos_of(callee).or_else(|| src_pos_of(caller)),
        _ => None,
    }
}

/// 0-based line of a location.
fn line_of(loc: &Location) -> Option<u32> {
    src_pos_of(loc).map(|(l, _)| (l.max(1) as u32) - 1)
}

/// Record what the parsed module knows about `op` and everything below it.
fn collect_op(ctx: &Context, op: Ptr<Operation>, facts: &mut Facts) {
    let line = line_of(op.deref(ctx).loc()).unwrap_or(0);
    let opid = Operation::get_opid(op, ctx).to_string();

    let mut results_sig = Vec::new();
    {
        let opref = op.deref(ctx);
        for res in opref.results() {
            let ty = res.get_type(ctx).disp(ctx).to_string().trim().to_string();
            match res.given_name(ctx).map(|i| i.to_string()) {
                Some(name) => {
                    facts.values.insert(
                        (name.clone(), line),
                        ValueFact {
                            ty: ty.clone(),
                            origin: format!("result of `{opid}`"),
                        },
                    );
                    results_sig.push(format!("{name}: {ty}"));
                }
                None => results_sig.push(ty),
            }
        }
    }

    let mut attrs = Vec::new();
    {
        let opref = op.deref(ctx);
        for (key, attr) in opref.attributes.0.iter() {
            if *key == ATTR_KEY_GIVEN_NAMES {
                continue;
            }
            let key = key.to_string();
            attrs.push(format!("{key}: {}", attr.disp(ctx).to_string().trim()));
        }
    }

    let sym_name = op
        .deref(ctx)
        .attributes
        .get::<IdentifierAttr>(&ATTR_KEY_SYM_NAME)
        .map(|a| Identifier::from(a.clone()).to_string());
    let mut hover = match &sym_name {
        Some(name) => format!("`{opid} @{name}`"),
        None => format!("`{opid}`"),
    };
    if !results_sig.is_empty() {
        hover.push_str(&format!("\n\nresults: `{}`", results_sig.join(", ")));
    }
    if !attrs.is_empty() {
        hover.push_str("\n\nattributes:\n");
        for a in &attrs {
            hover.push_str(&format!("- `{a}`\n"));
        }
    }
    facts.ops.insert((line, opid), hover);

    let regions: Vec<_> = op.deref(ctx).regions().collect();
    for region in regions {
        let blocks: Vec<_> = region.deref(ctx).iter(ctx).collect();
        for block in blocks {
            let bline = line_of(block.deref(ctx).loc()).unwrap_or(line);
            let label = block.deref(ctx).given_name(ctx).map(|i| i.to_string());
            let mut args_sig = Vec::new();
            {
                let bref = block.deref(ctx);
                for arg in bref.arguments() {
                    let ty = arg.get_type(ctx).disp(ctx).to_string().trim().to_string();
                    match arg.given_name(ctx).map(|i| i.to_string()) {
                        Some(name) => {
                            facts.values.insert(
                                (name.clone(), bline),
                                ValueFact {
                                    ty: ty.clone(),
                                    origin: format!(
                                        "block argument of `^{}`",
                                        label.as_deref().unwrap_or("<unnamed>")
                                    ),
                                },
                            );
                            args_sig.push(format!("{name}: {ty}"));
                        }
                        None => args_sig.push(ty),
                    }
                }
            }
            if let Some(label) = &label {
                facts.blocks.insert(
                    (label.clone(), bline),
                    format!("^{label}({})", args_sig.join(", ")),
                );
            }
            let ops: Vec<_> = block.deref(ctx).iter(ctx).collect();
            for child in ops {
                collect_op(ctx, child, facts);
            }
        }
    }
}
