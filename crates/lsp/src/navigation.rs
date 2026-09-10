//! Token-level definition/reference index for SSA value names and block
//! labels.
//!
//! pliron's in-memory IR does not retain source spans for individual
//! value *uses* (operations and blocks carry only their own start
//! [`Location`](pliron::location::Location)). We therefore build a
//! lexical index over the document text, keyed off the successfully
//! parsed module:
//!
//! * a scope tree is derived from `{`/`}` nesting (each region body is a
//!   scope);
//! * value definitions are identifiers at statement level followed by
//!   `=` (e.g. `sum = llvm.add ...`), block-argument definitions are the
//!   identifiers in a block header `^bb(a: T, b: T):`, block-label
//!   definitions are the `^bb` header tokens themselves;
//! * when the module parsed successfully, value/argument definitions are
//!   additionally filtered against the set of value names the parsed
//!   module actually defines, which discards look-alikes such as
//!   `variadic = false` inside type syntax;
//! * every other bare identifier is a candidate reference and resolves
//!   to the nearest preceding definition of that name in the innermost
//!   enclosing scope (a lexical approximation of SSA dominance); block
//!   labels resolve order-independently within their region.

use std::collections::{HashMap, HashSet};

use crate::lex::{Token, TokKind, lex};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DefKind {
    /// An op result (`sum = ...`).
    Value,
    /// A block argument (`^bb(a: T)`).
    BlockArg,
    /// A block label (`^bb(...):` header).
    BlockLabel,
    /// A symbol definition (`@name` on the line of a symbol-defining op).
    Symbol,
}

#[derive(Clone, Debug)]
pub struct Def {
    pub name: String,
    pub kind: DefKind,
    /// Index into `NavIndex::tokens`.
    pub token: usize,
}

struct Node {
    parent: Option<usize>,
    /// name -> defs (Value/BlockArg), in document order.
    values: HashMap<String, Vec<usize>>,
    /// label -> def, first one wins.
    labels: HashMap<String, usize>,
}

pub struct NavIndex {
    pub tokens: Vec<Token>,
    pub defs: Vec<Def>,
    /// (token index, def index) for every resolved reference.
    pub refs: Vec<(usize, usize)>,
    /// token index -> def index, for definition lookups (includes the
    /// defs' own tokens, mapping to themselves).
    token_to_def: HashMap<usize, usize>,
}

/// Optional facts from a successfully parsed module used to filter and
/// classify lexical matches.
pub struct ModuleNameFilter {
    /// All value names (op results and block args) defined in the module.
    pub value_names: HashSet<String>,
    /// All block labels in the module.
    pub block_labels: HashSet<String>,
    /// 0-based lines on which a symbol-defining op starts, keyed by
    /// symbol name.
    pub symbol_def_lines: HashMap<String, usize>,
}

impl NavIndex {
    pub fn build(text: &str, filter: Option<&ModuleNameFilter>) -> NavIndex {
        let tokens = lex(text);
        Builder::new(text, &tokens, filter).run()
    }

    /// The token containing byte `offset`, if it is a name token that
    /// participates in navigation.
    pub fn def_at(&self, offset: usize) -> Option<(usize, &Def)> {
        for (ti, tok) in self.tokens.iter().enumerate() {
            if tok.start > offset {
                break;
            }
            if tok.contains(offset) {
                return self.token_to_def.get(&ti).map(|&d| (ti, &self.defs[d]));
            }
        }
        None
    }

    /// All reference tokens (excluding the def token) of a definition.
    pub fn refs_of(&self, def_idx: usize) -> impl Iterator<Item = &Token> + '_ {
        self.refs
            .iter()
            .filter(move |(_, d)| *d == def_idx)
            .map(|(t, _)| &self.tokens[*t])
    }

    pub fn def_index_of(&self, def: &Def) -> Option<usize> {
        self.token_to_def.get(&def.token).copied()
    }
}

struct Builder<'a> {
    text: &'a str,
    tokens: &'a [Token],
    filter: Option<&'a ModuleNameFilter>,
    line_starts: Vec<usize>,
    nodes: Vec<Node>,
    /// (token, node) candidate value/label references, resolved at the end.
    cand_values: Vec<(usize, usize)>,
    cand_labels: Vec<(usize, usize)>,
    defs: Vec<Def>,
    sym_defs: HashMap<String, usize>,
    sym_refs: Vec<usize>,
}

impl<'a> Builder<'a> {
    fn new(text: &'a str, tokens: &'a [Token], filter: Option<&'a ModuleNameFilter>) -> Self {
        let mut line_starts = vec![0usize];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        Builder {
            text,
            tokens,
            filter,
            line_starts,
            nodes: vec![Node {
                parent: None,
                values: HashMap::new(),
                labels: HashMap::new(),
            }],
            cand_values: Vec::new(),
            cand_labels: Vec::new(),
            defs: Vec::new(),
            sym_defs: HashMap::new(),
            sym_refs: Vec::new(),
        }
    }

    fn line_of(&self, offset: usize) -> usize {
        match self.line_starts.binary_search(&offset) {
            Ok(l) => l,
            Err(l) => l - 1,
        }
    }

    fn value_name_ok(&self, name: &str) -> bool {
        match self.filter {
            Some(f) => f.value_names.contains(name),
            None => true,
        }
    }

    fn add_def(&mut self, node: usize, kind: DefKind, token: usize) {
        let name = self.tokens[token].name(self.text).to_string();
        let def_idx = self.defs.len();
        self.defs.push(Def {
            name: name.clone(),
            kind,
            token,
        });
        match kind {
            DefKind::Value | DefKind::BlockArg => {
                self.nodes[node].values.entry(name).or_default().push(def_idx)
            }
            DefKind::BlockLabel => {
                self.nodes[node].labels.entry(name).or_insert(def_idx);
            }
            DefKind::Symbol => {
                self.sym_defs.entry(name).or_insert(def_idx);
            }
        }
    }

    /// Find the token index of the `)` matching an `(` at `open`.
    fn matching_paren(&self, open: usize) -> Option<usize> {
        let mut depth = 0i32;
        for (j, t) in self.tokens.iter().enumerate().skip(open) {
            match t.kind {
                TokKind::Punct('(') => depth += 1,
                TokKind::Punct(')') => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(j);
                    }
                }
                // Don't run away across obvious statement boundaries.
                TokKind::Punct('{') | TokKind::Punct('}') if depth > 0 => {}
                _ => {}
            }
            // Guard against unbalanced input.
            if j > open + 4096 {
                break;
            }
        }
        None
    }

    fn run(mut self) -> NavIndex {
        let mut cur_node = 0usize;
        let mut angle = 0i32;
        let mut at_stmt_start = true;
        // Tokens already consumed as part of a def group / header.
        let mut consumed = vec![false; self.tokens.len()];

        let mut i = 0usize;
        while i < self.tokens.len() {
            let tok = self.tokens[i];
            match tok.kind {
                TokKind::Punct('{') => {
                    if angle == 0 {
                        self.nodes.push(Node {
                            parent: Some(cur_node),
                            values: HashMap::new(),
                            labels: HashMap::new(),
                        });
                        cur_node = self.nodes.len() - 1;
                        at_stmt_start = true;
                    }
                }
                TokKind::Punct('}') => {
                    if angle == 0 {
                        if let Some(p) = self.nodes[cur_node].parent {
                            cur_node = p;
                        }
                        at_stmt_start = true;
                    }
                }
                TokKind::Punct('<') => angle += 1,
                TokKind::Punct('>') => angle = (angle - 1).max(0),
                TokKind::Punct(';') => {
                    at_stmt_start = true;
                    angle = 0;
                }
                TokKind::Block if angle == 0 => {
                    // Header if `^bb ( ... ) :` or `^bb :`.
                    let mut header_end = None;
                    match self.tokens.get(i + 1).map(|t| t.kind) {
                        Some(TokKind::Punct('(')) => {
                            if let Some(close) = self.matching_paren(i + 1) {
                                if self.tokens.get(close + 1).map(|t| t.kind)
                                    == Some(TokKind::Punct(':'))
                                {
                                    header_end = Some((i + 1, close));
                                }
                            }
                        }
                        Some(TokKind::Punct(':')) => header_end = Some((i, i)),
                        _ => {}
                    }
                    if let Some((open, close)) = header_end {
                        self.add_def(cur_node, DefKind::BlockLabel, i);
                        // Block arguments: idents at header paren depth 1
                        // immediately followed by `:`.
                        let mut depth = 0i32;
                        let mut j = open;
                        while j <= close {
                            match self.tokens[j].kind {
                                TokKind::Punct('(') => depth += 1,
                                TokKind::Punct(')') => depth -= 1,
                                TokKind::Ident
                                    if depth == 1
                                        && self.tokens.get(j + 1).map(|t| t.kind)
                                            == Some(TokKind::Punct(':'))
                                        && matches!(
                                            self.tokens.get(j - 1).map(|t| t.kind),
                                            Some(TokKind::Punct('(') | TokKind::Punct(','))
                                        ) =>
                                {
                                    if self.value_name_ok(self.tokens[j].name(self.text)) {
                                        self.add_def(cur_node, DefKind::BlockArg, j);
                                    }
                                    consumed[j] = true;
                                }
                                _ => {}
                            }
                            j += 1;
                        }
                        for t in i..=close + 1 {
                            if t < consumed.len() {
                                consumed[t] = true;
                            }
                        }
                        i = close + 2;
                        at_stmt_start = true;
                        continue;
                    } else {
                        // A successor reference.
                        self.cand_labels.push((i, cur_node));
                    }
                }
                TokKind::Ident if angle == 0 => {
                    if at_stmt_start {
                        // `a = ...` or `a, b = ...` result group?
                        let mut group = vec![i];
                        let mut j = i;
                        while self.tokens.get(j + 1).map(|t| t.kind) == Some(TokKind::Punct(','))
                            && self.tokens.get(j + 2).map(|t| t.kind) == Some(TokKind::Ident)
                        {
                            group.push(j + 2);
                            j += 2;
                        }
                        let is_def_group = self.tokens.get(j + 1).map(|t| t.kind)
                            == Some(TokKind::Punct('='))
                            && self.tokens.get(j + 2).map(|t| t.kind)
                                != Some(TokKind::Punct('='));
                        if is_def_group {
                            for &g in &group {
                                if self.value_name_ok(self.tokens[g].name(self.text)) {
                                    self.add_def(cur_node, DefKind::Value, g);
                                }
                                consumed[g] = true;
                            }
                            i = j + 2; // skip past `=`
                            at_stmt_start = false;
                            continue;
                        }
                    }
                    if !consumed[i] {
                        let name = tok.name(self.text);
                        if name != "true" && name != "false" {
                            self.cand_values.push((i, cur_node));
                        }
                    }
                }
                TokKind::Sym => {
                    let name = tok.name(self.text).to_string();
                    let is_def = match self.filter {
                        Some(f) => f
                            .symbol_def_lines
                            .get(&name)
                            .is_some_and(|&l| l == self.line_of(tok.start)),
                        // Without a parsed module: first occurrence wins.
                        None => !self.sym_defs.contains_key(&name),
                    };
                    if is_def {
                        self.add_def(cur_node, DefKind::Symbol, i);
                    } else {
                        self.sym_refs.push(i);
                    }
                }
                _ => {}
            }
            if !matches!(tok.kind, TokKind::Punct('{') | TokKind::Punct('}')) {
                at_stmt_start = matches!(tok.kind, TokKind::Punct(';'));
            }
            i += 1;
        }

        self.resolve(consumed)
    }

    fn resolve(self, _consumed: Vec<bool>) -> NavIndex {
        let mut refs: Vec<(usize, usize)> = Vec::new();
        let mut token_to_def: HashMap<usize, usize> = HashMap::new();

        for (d, def) in self.defs.iter().enumerate() {
            token_to_def.insert(def.token, d);
        }

        // Value references: nearest preceding def in the innermost scope;
        // fall back to any def of the name in an enclosing scope (to keep
        // navigation working in graph regions without dominance).
        for &(ti, node) in &self.cand_values {
            let offset = self.tokens[ti].start;
            let name = self.tokens[ti].name(self.text);
            let mut resolved = None;
            let mut n = Some(node);
            while let Some(cur) = n {
                if let Some(defs) = self.nodes[cur].values.get(name) {
                    // defs are in document order.
                    let before = defs
                        .iter()
                        .rev()
                        .find(|&&d| self.tokens[self.defs[d].token].start <= offset);
                    resolved = before.or_else(|| defs.first()).copied();
                    if resolved.is_some() {
                        break;
                    }
                }
                n = self.nodes[cur].parent;
            }
            if let Some(d) = resolved {
                refs.push((ti, d));
                token_to_def.insert(ti, d);
            }
        }

        // Label references: order-independent within enclosing scopes.
        for &(ti, node) in &self.cand_labels {
            let name = self.tokens[ti].name(self.text);
            let mut n = Some(node);
            while let Some(cur) = n {
                if let Some(&d) = self.nodes[cur].labels.get(name) {
                    refs.push((ti, d));
                    token_to_def.insert(ti, d);
                    break;
                }
                n = self.nodes[cur].parent;
            }
        }

        // Symbol references: module-wide.
        for &ti in &self.sym_refs {
            let name = self.tokens[ti].name(self.text);
            if let Some(&d) = self.sym_defs.get(name) {
                refs.push((ti, d));
                token_to_def.insert(ti, d);
            }
        }

        NavIndex {
            tokens: self.tokens.to_vec(),
            defs: self.defs,
            refs,
            token_to_def,
        }
    }
}
