//! Analysis tests against real llvm-dialect IR (linked via the
//! `pliron-llvm` dev-dependency; its dialects self-register at link
//! time).

use pliron_inspect_lsp::DocumentAnalysis;
use pliron_llvm as _;

use lsp_types::Position;

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

fn pos_of(text: &str, needle: &str, occurrence: usize) -> Position {
    let mut idx = 0;
    let mut found = 0;
    loop {
        let at = text[idx..].find(needle).expect("needle present") + idx;
        found += 1;
        if found > occurrence {
            let line = text[..at].bytes().filter(|&b| b == b'\n').count();
            let line_start = text[..at].rfind('\n').map(|i| i + 1).unwrap_or(0);
            return Position {
                line: line as u32,
                character: (at - line_start) as u32,
            };
        }
        idx = at + needle.len();
    }
}

#[test]
fn valid_module_has_no_diagnostics() {
    let a = DocumentAnalysis::new(VALID.to_string());
    assert!(a.parsed, "module must parse");
    assert!(
        a.diagnostics.is_empty(),
        "expected no diagnostics, got: {:?}",
        a.diagnostics
    );
}

#[test]
fn document_symbols_list_module_func_and_blocks() {
    let a = DocumentAnalysis::new(VALID.to_string());
    assert_eq!(a.symbols.len(), 1);
    let module = &a.symbols[0];
    assert_eq!(module.name, "@m");
    let children = module.children.as_ref().expect("module children");
    let func = children
        .iter()
        .find(|c| c.name == "@foo")
        .expect("func symbol");
    let func_children = func.children.as_ref().expect("func children");
    assert!(
        func_children.iter().any(|c| c.name == "^entry_block_1_0"),
        "blocks listed as children: {func_children:?}"
    );
}

#[test]
fn definition_of_value_reference() {
    let a = DocumentAnalysis::new(VALID.to_string());
    // The `sum` in `llvm.return sum` resolves to `sum = llvm.add ...`.
    let use_pos = pos_of(VALID, "sum", 1);
    let def_range = a.definition(use_pos).expect("definition found");
    let def_pos = pos_of(VALID, "sum", 0);
    assert_eq!(def_range.start, def_pos);

    // The `a` in `llvm.add a, b` resolves to its constant def.
    let use_pos = pos_of(VALID, "llvm.add a", 0);
    let a_use = Position {
        line: use_pos.line,
        character: use_pos.character + "llvm.add ".len() as u32,
    };
    let def_range = a.definition(a_use).expect("definition of a");
    assert_eq!(def_range.start, pos_of(VALID, "a = builtin.constant", 0));
}

#[test]
fn references_of_value() {
    let a = DocumentAnalysis::new(VALID.to_string());
    let def_pos = pos_of(VALID, "sum", 0);
    let refs = a.references(def_pos, true);
    assert_eq!(refs.len(), 2, "def + one use: {refs:?}");
}

#[test]
fn hover_on_value_shows_type() {
    let a = DocumentAnalysis::new(VALID.to_string());
    let (_, text) = a.hover(pos_of(VALID, "sum", 0)).expect("hover");
    assert!(
        text.contains("builtin.integer") && text.contains("i64"),
        "hover shows the type: {text}"
    );
    assert!(text.contains("llvm.add"), "hover names the def op: {text}");
}

#[test]
fn hover_on_op_name() {
    let a = DocumentAnalysis::new(VALID.to_string());
    let (_, text) = a.hover(pos_of(VALID, "llvm.add", 0)).expect("op hover");
    assert!(text.contains("llvm.add"), "op hover: {text}");
}

#[test]
fn broken_ir_produces_diagnostic_with_position() {
    let broken = "builtin.module @m {\n^b():\n  llvm.bogus_op x\n}\n";
    let a = DocumentAnalysis::new(broken.to_string());
    assert!(!a.diagnostics.is_empty(), "diagnostic expected");
    let d = &a.diagnostics[0];
    assert!(
        d.range.start.line >= 1,
        "range points into the document: {:?}",
        d.range
    );
}

#[test]
fn block_label_definition_and_references() {
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
    let a = DocumentAnalysis::new(ir.to_string());
    // Even if this exact cond_br syntax fails to parse, lexical navigation
    // must still resolve the label.
    let use_pos = pos_of(ir, "^left ()", 0);
    let def_range = a.definition(use_pos).expect("label definition");
    assert_eq!(def_range.start, pos_of(ir, "^left():", 0));
}
