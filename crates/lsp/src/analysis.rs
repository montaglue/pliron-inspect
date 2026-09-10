//! Per-document analysis: parse the text into its own fresh pliron
//! [`Context`] as a single module, verify it, and derive diagnostics,
//! document symbols, hover information and a navigation index.
//!
//! Everything here is strictly intra-module: there is no workspace
//! indexing and no cross-file resolution (see the crate docs).

use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};

use pliron::builtin::attributes::IdentifierAttr;
use pliron::builtin::op_interfaces::ATTR_KEY_SYM_NAME;
use pliron::common_traits::Named;
use pliron::context::{Context, Ptr};
use pliron::identifier::Identifier;
use pliron::irfmt::parsers::spaced;
use pliron::linked_list::ContainsLinkedList;
use pliron::location::{Located, Location};
use pliron::operation::{Operation, verify_operation};
use pliron::parsable::parse_from_str;
use pliron::printable::Printable;
use pliron::r#type::Typed;

use lsp_types::{
    Diagnostic, DiagnosticSeverity, DocumentSymbol, Position, Range, SymbolKind,
};

use crate::lex::{TokKind, Token};
use crate::lineindex::LineIndex;
use crate::navigation::{Def, DefKind, ModuleNameFilter, NavIndex};

/// Facts about a value defined by the parsed module, keyed by
/// `(name, 0-based def line)`.
struct ValueFact {
    type_str: String,
    /// "block argument of ^bb" or "result of dialect.op".
    origin: String,
}

/// Facts about one operation of the parsed module.
struct OpFact {
    /// 0-based line of the op's start.
    line: usize,
    opid: String,
    hover: String,
}

#[derive(Default)]
struct ModuleFacts {
    values: HashMap<(String, usize), ValueFact>,
    /// (label, line) -> rendered block signature.
    blocks: HashMap<(String, usize), String>,
    ops: Vec<OpFact>,
    filter: ModuleNameFilter,
    symbols: Vec<DocumentSymbol>,
}

impl Default for ModuleNameFilter {
    fn default() -> Self {
        ModuleNameFilter {
            value_names: HashSet::new(),
            block_labels: HashSet::new(),
            symbol_def_lines: HashMap::new(),
        }
    }
}

/// The complete analysis of one document.
pub struct DocumentAnalysis {
    pub text: String,
    pub line_index: LineIndex,
    pub diagnostics: Vec<Diagnostic>,
    pub symbols: Vec<DocumentSymbol>,
    nav: NavIndex,
    /// Precomputed hover markdown per definition index.
    def_hover: HashMap<usize, String>,
    /// (line, opid) -> hover markdown, for hovering op-name tokens.
    op_hover: HashMap<(usize, String), String>,
    /// Did the document parse into a module?
    pub parsed: bool,
}

impl DocumentAnalysis {
    pub fn new(text: String) -> DocumentAnalysis {
        let line_index = LineIndex::new(&text);
        let mut diagnostics = Vec::new();

        // Parse (and verify) in a fresh context, guarding against panics
        // in dialect parser/verifier code.
        let parse_result = catch_unwind(AssertUnwindSafe(|| {
            let mut ctx = Context::new();
            let res = parse_from_str(
                spaced(Operation::top_level_parser()),
                &mut ctx,
                text.as_str(),
            );
            (ctx, res)
        }));

        let mut facts = ModuleFacts::default();
        let mut parsed = false;

        match parse_result {
            Ok((ctx, Ok(op))) => {
                parsed = true;
                // Verifier diagnostics (best-effort ranges: the location
                // recorded on the offending entity, which is its start).
                let verify_result =
                    catch_unwind(AssertUnwindSafe(|| verify_operation(op, &ctx)));
                match verify_result {
                    Ok(Ok(())) => {}
                    Ok(Err(err)) => {
                        let (start, end) =
                            location_to_span(&err.loc, &line_index, &text);
                        diagnostics.push(Diagnostic {
                            range: line_index.range_of(start, end, &text),
                            severity: Some(DiagnosticSeverity::ERROR),
                            source: Some("pliron-verify".into()),
                            message: strip_location_prefix(
                                &err.disp(&ctx).to_string(),
                            ),
                            ..Diagnostic::default()
                        });
                    }
                    Err(panic) => diagnostics.push(Diagnostic {
                        range: whole_first_line(&line_index, &text),
                        severity: Some(DiagnosticSeverity::ERROR),
                        source: Some("pliron-verify".into()),
                        message: format!("verifier panicked: {}", panic_msg(panic)),
                        ..Diagnostic::default()
                    }),
                }
                facts = collect_module_facts(&ctx, op, &line_index, &text);
            }
            Ok((ctx, Err(err))) => {
                let (start, end) = location_to_span(&err.loc, &line_index, &text);
                diagnostics.push(Diagnostic {
                    range: line_index.range_of(start, end, &text),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("pliron-parse".into()),
                    message: strip_location_prefix(&err.disp(&ctx).to_string()),
                    ..Diagnostic::default()
                });
            }
            Err(panic) => diagnostics.push(Diagnostic {
                range: whole_first_line(&line_index, &text),
                severity: Some(DiagnosticSeverity::ERROR),
                source: Some("pliron-parse".into()),
                message: format!("parser panicked: {}", panic_msg(panic)),
                ..Diagnostic::default()
            }),
        }

        let nav = NavIndex::build(&text, if parsed { Some(&facts.filter) } else { None });

        // Precompute hover text per definition.
        let mut def_hover = HashMap::new();
        for (d, def) in nav.defs.iter().enumerate() {
            let tok = nav.tokens[def.token];
            let line = line_index.line_of(tok.start);
            let hover = match def.kind {
                DefKind::Value | DefKind::BlockArg => facts
                    .values
                    .get(&(def.name.clone(), line))
                    .map(|f| {
                        format!(
                            "`{}: {}`\n\n{} (line {})",
                            def.name,
                            f.type_str.trim(),
                            f.origin,
                            line + 1
                        )
                    })
                    .unwrap_or_else(|| {
                        format!("`{}` — SSA value (line {})", def.name, line + 1)
                    }),
                DefKind::BlockLabel => facts
                    .blocks
                    .get(&(def.name.clone(), line))
                    .map(|sig| format!("`{}`\n\nblock (line {})", sig, line + 1))
                    .unwrap_or_else(|| {
                        format!("`^{}` — block (line {})", def.name, line + 1)
                    }),
                DefKind::Symbol => {
                    let opid = facts
                        .ops
                        .iter()
                        .find(|o| o.line == line)
                        .map(|o| o.opid.clone())
                        .unwrap_or_else(|| "op".into());
                    format!("`@{}`\n\nsymbol defined by `{}` (line {})", def.name, opid, line + 1)
                }
            };
            def_hover.insert(d, hover);
        }

        let op_hover = facts
            .ops
            .iter()
            .map(|o| ((o.line, o.opid.clone()), o.hover.clone()))
            .collect();

        DocumentAnalysis {
            text,
            line_index,
            diagnostics,
            symbols: facts.symbols,
            nav,
            def_hover,
            op_hover,
            parsed,
        }
    }

    fn token_range(&self, tok: &Token) -> Range {
        self.line_index.range_of(tok.start, tok.end, &self.text)
    }

    /// Definition site of the name at `pos`, if any.
    pub fn definition(&self, pos: Position) -> Option<Range> {
        let offset = self.line_index.offset_of(pos, &self.text);
        let (_, def) = self.nav.def_at(offset)?;
        let tok = self.nav.tokens[def.token];
        Some(self.token_range(&tok))
    }

    /// All references (optionally including the declaration) of the name
    /// at `pos`.
    pub fn references(&self, pos: Position, include_decl: bool) -> Vec<Range> {
        let offset = self.line_index.offset_of(pos, &self.text);
        let Some((_, def)) = self.nav.def_at(offset) else {
            return vec![];
        };
        let Some(def_idx) = self.nav.def_index_of(def) else {
            return vec![];
        };
        let mut out = Vec::new();
        if include_decl {
            out.push(self.token_range(&self.nav.tokens[def.token]));
        }
        for tok in self.nav.refs_of(def_idx) {
            out.push(self.token_range(tok));
        }
        out
    }

    /// Hover for the token at `pos`: values/labels/symbols show their
    /// definition info; op-name tokens show the op signature.
    pub fn hover(&self, pos: Position) -> Option<(Range, String)> {
        let offset = self.line_index.offset_of(pos, &self.text);
        // A value / label / symbol name?
        if let Some((ti, def)) = self.nav.def_at(offset) {
            let def_idx = self.nav.def_index_of(def)?;
            let text = self.def_hover.get(&def_idx)?.clone();
            return Some((self.token_range(&self.nav.tokens[ti]), text));
        }
        // An op name (dotted identifier)?
        let tok = self
            .nav
            .tokens
            .iter()
            .find(|t| t.contains(offset) && t.kind == TokKind::DottedIdent)?;
        let line = self.line_index.line_of(tok.start);
        let opid = tok.text(&self.text).to_string();
        let hover = self.op_hover.get(&(line, opid))?;
        Some((self.token_range(tok), hover.clone()))
    }

    pub fn nav_def_at(&self, pos: Position) -> Option<&Def> {
        let offset = self.line_index.offset_of(pos, &self.text);
        self.nav.def_at(offset).map(|(_, d)| d)
    }
}

fn panic_msg(panic: Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".to_string())
}

fn whole_first_line(li: &LineIndex, text: &str) -> Range {
    let (s, e) = li.line_range(0, text);
    li.range_of(s, e, text)
}

/// pliron error messages rendered with `disp` may start with a
/// `<in-memory>: line: N, column: M\n` prefix; the position is already
/// carried by the diagnostic range, so strip it.
fn strip_location_prefix(msg: &str) -> String {
    let msg = msg.trim();
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

/// Best-effort byte span for a pliron [`Location`]: the location's
/// position through the end of the token that starts there (or a single
/// character).
fn location_to_span(loc: &Location, li: &LineIndex, text: &str) -> (usize, usize) {
    match src_pos_of(loc) {
        Some((line, col)) => {
            let start = li.offset_of_src_pos(line, col, text);
            // Extend to the end of the current line's trimmed content, or
            // at least one character.
            let l = li.line_of(start);
            let (_, line_end) = li.line_range(l, text);
            let end = if line_end > start { line_end } else { (start + 1).min(text.len()) };
            (start, end)
        }
        None => {
            let (s, e) = li.line_range(0, text);
            (s, e)
        }
    }
}

fn src_pos_of(loc: &Location) -> Option<(i32, i32)> {
    match loc {
        Location::SrcPos { pos, .. } => Some((pos.line, pos.column)),
        Location::Fused { locations, .. } => locations.iter().find_map(src_pos_of),
        Location::Named { child_loc, .. } => src_pos_of(child_loc),
        Location::CallSite { callee, caller } => {
            src_pos_of(callee).or_else(|| src_pos_of(caller))
        }
        _ => None,
    }
}

fn loc_line0(loc: &Location) -> Option<usize> {
    src_pos_of(loc).map(|(l, _)| (l.max(1) as usize) - 1)
}

fn collect_module_facts(
    ctx: &Context,
    root: Ptr<Operation>,
    li: &LineIndex,
    text: &str,
) -> ModuleFacts {
    let mut facts = ModuleFacts::default();
    let root_sym = collect_op(ctx, root, li, text, &mut facts, 0);
    if let Some(sym) = root_sym {
        facts.symbols.push(sym);
    }
    facts
}

/// Recursively collect facts for `op`. Returns a `DocumentSymbol` for the
/// op when it defines a symbol name.
fn collect_op(
    ctx: &Context,
    op: Ptr<Operation>,
    li: &LineIndex,
    text: &str,
    facts: &mut ModuleFacts,
    depth: usize,
) -> Option<DocumentSymbol> {
    let loc = op.deref(ctx).loc();
    let line = loc_line0(&loc).unwrap_or(0);
    let start = src_pos_of(&loc)
        .map(|(l, c)| li.offset_of_src_pos(l, c, text))
        .unwrap_or(0);
    let opid = Operation::get_opid(op, ctx).to_string();

    // Result facts.
    let mut results_sig = Vec::new();
    {
        let opref = op.deref(ctx);
        for res in opref.results() {
            let name = res.given_name(ctx).map(|i| i.to_string());
            let ty = res.get_type(ctx).disp(ctx).to_string().trim().to_string();
            if let Some(name) = name {
                facts.filter.value_names.insert(name.clone());
                facts.values.insert(
                    (name.clone(), line),
                    ValueFact {
                        type_str: ty.clone(),
                        origin: format!("result of `{opid}`"),
                    },
                );
                results_sig.push(format!("{name}: {ty}"));
            } else {
                results_sig.push(ty);
            }
        }
    }

    // Attribute rendering (skip internal debug-info names).
    let mut attrs = Vec::new();
    {
        let opref = op.deref(ctx);
        for (key, attr) in opref.attributes.0.iter() {
            let key_s = key.to_string();
            if key_s == "builtin_debug_info" {
                continue;
            }
            attrs.push(format!("{}: {}", key_s, attr.disp(ctx).to_string().trim()));
        }
    }

    let mut hover = format!("`{opid}`");
    if !results_sig.is_empty() {
        hover.push_str(&format!("\n\nresults: `{}`", results_sig.join(", ")));
    }
    if !attrs.is_empty() {
        hover.push_str("\n\nattributes:\n");
        for a in &attrs {
            hover.push_str(&format!("- `{a}`\n"));
        }
    }

    // Symbol name, if this op defines one (the `sym_name` attribute used
    // by `SymbolOpInterface`).
    let sym_name = {
        let opref = op.deref(ctx);
        opref
            .attributes
            .get::<IdentifierAttr>(&ATTR_KEY_SYM_NAME)
            .map(|a| Identifier::from(a.clone()).to_string())
    };
    if let Some(ref name) = sym_name {
        facts.filter.symbol_def_lines.insert(name.clone(), line);
        hover = format!("`{opid} @{name}`{}", hover.trim_start_matches(&format!("`{opid}`")));
    }

    facts.ops.push(OpFact {
        line,
        opid: opid.clone(),
        hover,
    });

    // Recurse into regions and blocks.
    let regions: Vec<_> = op.deref(ctx).regions().collect();
    let mut child_symbols = Vec::new();
    let mut last_line = line;
    for region in regions {
        let blocks: Vec<_> = region.deref(ctx).iter(ctx).collect();
        for block in blocks {
            let bloc = block.deref(ctx).loc();
            let bline = loc_line0(&bloc).unwrap_or(line);
            last_line = last_line.max(bline);
            let label = block.deref(ctx).given_name(ctx).map(|i| i.to_string());
            // Block signature and argument facts.
            let mut args_sig = Vec::new();
            {
                let bref = block.deref(ctx);
                for arg in bref.arguments() {
                    let name = arg.given_name(ctx).map(|i| i.to_string());
                    let ty = arg.get_type(ctx).disp(ctx).to_string().trim().to_string();
                    if let Some(name) = name {
                        facts.filter.value_names.insert(name.clone());
                        facts.values.insert(
                            (name.clone(), bline),
                            ValueFact {
                                type_str: ty.clone(),
                                origin: format!(
                                    "block argument of `^{}`",
                                    label.as_deref().unwrap_or("<unnamed>")
                                ),
                            },
                        );
                        args_sig.push(format!("{name}: {ty}"));
                    } else {
                        args_sig.push(ty);
                    }
                }
            }
            if let Some(ref label) = label {
                facts.filter.block_labels.insert(label.clone());
                facts
                    .blocks
                    .insert((label.clone(), bline), format!("^{}({})", label, args_sig.join(", ")));
                #[allow(deprecated)]
                child_symbols.push(DocumentSymbol {
                    name: format!("^{label}"),
                    detail: (!args_sig.is_empty()).then(|| format!("({})", args_sig.join(", "))),
                    kind: SymbolKind::NAMESPACE,
                    tags: None,
                    deprecated: None,
                    range: line_span_range(li, text, bline),
                    selection_range: line_span_range(li, text, bline),
                    children: None,
                });
            }
            let ops: Vec<_> = block.deref(ctx).iter(ctx).collect();
            for child in ops {
                if let Some(cline) = loc_line0(&child.deref(ctx).loc()) {
                    last_line = last_line.max(cline);
                }
                if let Some(sym) = collect_op(ctx, child, li, text, facts, depth + 1) {
                    child_symbols.push(sym);
                }
            }
        }
    }

    sym_name.map(|name| {
        let end_line = last_line.max(line);
        let (_, end_off) = li.line_range(end_line, text);
        let range = Range {
            start: li.position_of(start, text),
            end: li.position_of(end_off, text),
        };
        let kind = if !child_symbols.is_empty() || depth == 0 {
            if depth == 0 {
                SymbolKind::MODULE
            } else {
                SymbolKind::FUNCTION
            }
        } else if op.deref(ctx).num_regions() > 0 {
            SymbolKind::FUNCTION
        } else {
            SymbolKind::VARIABLE
        };
        #[allow(deprecated)]
        DocumentSymbol {
            name: format!("@{name}"),
            detail: Some(opid),
            kind,
            tags: None,
            deprecated: None,
            range,
            selection_range: line_span_range(li, text, line),
            children: (!child_symbols.is_empty()).then_some(child_symbols),
        }
    })
}

fn line_span_range(li: &LineIndex, text: &str, line: usize) -> Range {
    let (s, e) = li.line_range(line, text);
    li.range_of(s, e, text)
}
