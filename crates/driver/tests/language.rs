//! Language queries against real llvm-dialect IR (the LLVM dialect is
//! linked by the driver; it self-registers on `Context::new()`).

use pliron::context::Context;
use pliron_inspect_driver::language::Document;
use pliron_inspect_protocol::server::DocPosition;
use pliron_llvm as _;

const VALID: &str = r#"builtin.module @m {
^block_0_0():
  llvm.func @foo: llvm.func <builtin.integer i64() variadic = false> [] {
  ^entry_block_1_0():
    a = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64;
    b = llvm.constant <builtin.integer <4: i64>> : builtin.integer i64;
    sum = llvm.add a, b <{nsw=false,nuw=false}> : builtin.integer i64;
    llvm.return sum
  }
}
"#;

fn analyze(text: &str) -> Document {
    Document::analyze(text.to_string(), &mut Context::new())
}

/// The position of the `occurrence`-th `needle` (0-based).
fn pos_of(text: &str, needle: &str, occurrence: usize) -> DocPosition {
    let at = text
        .match_indices(needle)
        .nth(occurrence)
        .expect("needle present")
        .0;
    let line = text[..at].bytes().filter(|&b| b == b'\n').count();
    let line_start = text[..at].rfind('\n').map(|i| i + 1).unwrap_or(0);
    DocPosition {
        line: line as u32,
        character: (at - line_start) as u32,
    }
}

#[test]
fn valid_module_has_no_diagnostics() {
    let a = analyze(VALID);
    assert!(a.parsed, "module must parse");
    assert!(
        a.diagnostics.is_empty(),
        "expected no diagnostics, got: {:?}",
        a.diagnostics
    );
}

#[test]
fn definition_of_value_reference() {
    let a = analyze(VALID);
    // The `sum` in `llvm.return sum` resolves to `sum = llvm.add ...`.
    let def = a
        .definition(pos_of(VALID, "sum", 1))
        .expect("definition found");
    assert_eq!(def.start, pos_of(VALID, "sum", 0));
    // The `a` in `llvm.add a, b` resolves to its constant def.
    let use_pos = pos_of(VALID, "llvm.add a", 0);
    let a_use = DocPosition {
        line: use_pos.line,
        character: use_pos.character + "llvm.add ".len() as u32,
    };
    let def = a.definition(a_use).expect("definition of a");
    assert_eq!(def.start, pos_of(VALID, "a = builtin.constant", 0));
}

#[test]
fn references_of_value_from_definition_and_use() {
    let a = analyze(VALID);
    let refs = a.references(pos_of(VALID, "sum", 0), true);
    assert_eq!(refs.len(), 2, "def + one use: {refs:?}");
    // Asking from the use gives the same set.
    let refs = a.references(pos_of(VALID, "sum", 1), true);
    assert_eq!(refs.len(), 2, "{refs:?}");
    assert_eq!(refs[0].start, pos_of(VALID, "sum", 0), "definition first");
    assert_eq!(a.references(pos_of(VALID, "sum", 1), false).len(), 1);
}

#[test]
fn hover_on_value_shows_exact_type_and_defining_op() {
    let a = analyze(VALID);
    let (_, text) = a.hover(pos_of(VALID, "sum", 0)).expect("hover");
    assert!(
        text.contains("builtin.integer") && text.contains("i64"),
        "hover shows the type: {text}"
    );
    assert!(text.contains("llvm.add"), "hover names the def op: {text}");
    // Also from a use.
    let (range, text2) = a.hover(pos_of(VALID, "sum", 1)).expect("hover on use");
    assert_eq!(text, text2);
    assert_eq!(range.start, pos_of(VALID, "sum", 1));
}

#[test]
fn hover_on_op_name() {
    let a = analyze(VALID);
    let (_, text) = a.hover(pos_of(VALID, "llvm.add", 0)).expect("op hover");
    assert!(text.contains("llvm.add"), "op hover: {text}");
    assert!(text.contains("nsw"), "op hover lists attributes: {text}");
    let (_, text) = a.hover(pos_of(VALID, "llvm.func", 0)).expect("func hover");
    assert!(text.contains("@foo"), "symbol ops show their name: {text}");
}

#[test]
fn hover_on_symbol_and_label() {
    let a = analyze(VALID);
    let (_, text) = a.hover(pos_of(VALID, "@foo", 0)).expect("symbol hover");
    assert!(text.contains("llvm.func"), "{text}");
    let (_, text) = a
        .hover(pos_of(VALID, "^entry_block_1_0", 0))
        .expect("label hover");
    assert!(text.contains("^entry_block_1_0()"), "{text}");
}

#[test]
fn broken_ir_produces_a_positioned_parse_diagnostic() {
    let broken = "builtin.module @m {\n^b():\n  llvm.bogus_op x\n}\n";
    let a = analyze(broken);
    assert!(!a.parsed);
    let parse = a
        .diagnostics
        .iter()
        .find(|d| d.source == "pliron-parse")
        .expect("parse diagnostic");
    assert_eq!(
        parse.range.start.line, 2,
        "points at the op: {:?}",
        parse.range
    );
    assert!(
        parse.message.starts_with("Compilation error"),
        "location prefix stripped: {}",
        parse.message
    );
    assert!(parse.message.contains("llvm.bogus_op"), "{}", parse.message);
}

#[test]
fn syntax_diagnostics_locate_what_the_parser_only_stops_at() {
    // A missing `;` between two ops: pliron fails at the second op; the
    // syntax layer says what is missing and where.
    let ir = "builtin.module @m {\n^b():\n  x = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64\n  y = builtin.constant <builtin.integer <2: i64>> : builtin.integer i64\n}\n";
    let a = analyze(ir);
    assert!(!a.parsed);
    assert!(
        a.diagnostics
            .iter()
            .any(|d| d.source == "syntax" && d.message.contains(';')),
        "{:?}",
        a.diagnostics
    );
    // Navigation still works on the unparsed text.
    assert!(a.references(pos_of(ir, "x =", 0), true).len() == 1);
}

#[test]
fn verifier_errors_are_reported() {
    // Mismatched operand types: parses, fails verification.
    let ir = r#"builtin.module @m {
^block_0_0():
  llvm.func @foo: llvm.func <builtin.integer i64() variadic = false> [] {
  ^entry():
    a = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64;
    b = builtin.constant <builtin.integer <4: i32>> : builtin.integer i32;
    sum = llvm.add a, b <{nsw=false,nuw=false}> : builtin.integer i64;
    llvm.return sum
  }
}
"#;
    let a = analyze(ir);
    assert!(a.parsed, "{:?}", a.diagnostics);
    let verify = a
        .diagnostics
        .iter()
        .find(|d| d.source == "pliron-verify")
        .unwrap_or_else(|| panic!("{:?}", a.diagnostics));
    assert!(verify.range.start.line >= 1, "{:?}", verify.range);
}

#[test]
fn block_label_definition_and_references_without_a_parse() {
    let ir = r#"builtin.module @m {
^block_0_0():
  llvm.func @foo: llvm.func <builtin.integer i64(builtin.integer i1) variadic = false> [] {
  ^entry(c: builtin.integer i1):
    llvm.cond_br c [1, 0, 0] ^left () ^right ()
  ^left():
    x = llvm.constant <builtin.integer <1: i64>> : builtin.integer i64;
    llvm.return x
  ^right():
    y = llvm.constant <builtin.integer <2: i64>> : builtin.integer i64;
    llvm.return y
  }
}
"#;
    let a = analyze(ir);
    // Whether or not this cond_br syntax parses, navigation resolves the label.
    let def = a
        .definition(pos_of(ir, "^left ()", 0))
        .expect("label definition");
    assert_eq!(def.start, pos_of(ir, "^left():", 0));
    let refs = a.references(pos_of(ir, "^left():", 0), true);
    assert_eq!(refs.len(), 2, "{refs:?}");
}

#[test]
fn names_are_scoped_per_function() {
    // `x` in @g must not resolve to @f's `x`.
    let ir = r#"builtin.module @m {
^b():
  llvm.func @f: llvm.func <builtin.integer i64() variadic = false> [] {
  ^e():
    x = llvm.constant <builtin.integer <1: i64>> : builtin.integer i64;
    llvm.return x
  };
  llvm.func @g: llvm.func <builtin.integer i64() variadic = false> [] {
  ^e():
    x = llvm.constant <builtin.integer <2: i64>> : builtin.integer i64;
    llvm.return x
  }
}
"#;
    let a = analyze(ir);
    assert!(a.parsed, "{:?}", a.diagnostics);
    let at = pos_of(ir, "return x", 1);
    let x_use = DocPosition {
        line: at.line,
        character: at.character + "return ".len() as u32,
    };
    let def = a.definition(x_use).expect("def");
    assert_eq!(
        def.start,
        pos_of(ir, "x = llvm.constant <builtin.integer <2", 0)
    );
    assert_eq!(a.references(pos_of(ir, "x = llvm", 0), true).len(), 2);
}
