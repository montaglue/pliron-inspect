//! End-to-end test: speak LSP (JSON-RPC over stdio, Content-Length
//! framing) to the reference server binary.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

struct Lsp {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
}

impl Lsp {
    fn start() -> Lsp {
        let mut child = Command::new(env!("CARGO_BIN_EXE_pliron-inspect-lsp-server"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn server");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Lsp {
            child,
            stdin,
            stdout,
            next_id: 1,
        }
    }

    fn send(&mut self, msg: Value) {
        let body = serde_json::to_string(&msg).unwrap();
        write!(self.stdin, "Content-Length: {}\r\n\r\n{}", body.len(), body).unwrap();
        self.stdin.flush().unwrap();
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    fn request(&mut self, method: &str, params: Value) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        id
    }

    fn recv(&mut self) -> Value {
        let mut content_length = None;
        loop {
            let mut line = String::new();
            self.stdout.read_line(&mut line).expect("read header");
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some(v) = line.strip_prefix("Content-Length:") {
                content_length = Some(v.trim().parse::<usize>().unwrap());
            }
        }
        let len = content_length.expect("Content-Length header");
        let mut buf = vec![0u8; len];
        self.stdout.read_exact(&mut buf).expect("read body");
        serde_json::from_slice(&buf).expect("valid JSON body")
    }

    /// Read messages until the response with the given id arrives.
    fn wait_response(&mut self, id: i64) -> Value {
        loop {
            let msg = self.recv();
            if msg.get("id").and_then(Value::as_i64) == Some(id) && msg.get("method").is_none() {
                return msg;
            }
        }
    }

    /// Read messages until a publishDiagnostics notification for `uri`.
    fn wait_diagnostics(&mut self, uri: &str) -> Value {
        loop {
            let msg = self.recv();
            if msg.get("method").and_then(Value::as_str)
                == Some("textDocument/publishDiagnostics")
                && msg["params"]["uri"].as_str() == Some(uri)
            {
                return msg["params"]["diagnostics"].clone();
            }
        }
    }

    fn shutdown(mut self) {
        let id = self.request("shutdown", json!(null));
        self.wait_response(id);
        self.notify("exit", json!(null));
        let status = self.child.wait().expect("server exit");
        assert!(status.success(), "server exited cleanly");
    }
}

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

const BROKEN: &str = r#"builtin.module @m {
^block_0_0():
  llvm.func @foo: llvm.func <builtin.integer i64() variadic = false> [] {
  ^entry_block_1_0():
    a = llvm.totally_bogus ;
    llvm.return a
  }
}
"#;

#[test]
fn end_to_end_over_stdio() {
    let mut lsp = Lsp::start();

    // initialize / initialized.
    let id = lsp.request("initialize", json!({"capabilities": {}}));
    let resp = lsp.wait_response(id);
    let caps = &resp["result"]["capabilities"];
    assert!(caps["definitionProvider"].as_bool().unwrap_or(false));
    assert!(caps["documentSymbolProvider"].as_bool().unwrap_or(false));
    lsp.notify("initialized", json!({}));

    // (a) valid llvm-dialect IR: zero diagnostics.
    let valid_uri = "file:///test/valid.plir";
    lsp.notify(
        "textDocument/didOpen",
        json!({"textDocument": {
            "uri": valid_uri, "languageId": "plir", "version": 1, "text": VALID}}),
    );
    let diags = lsp.wait_diagnostics(valid_uri);
    assert_eq!(
        diags.as_array().map(Vec::len),
        Some(0),
        "valid IR must have zero diagnostics: {diags}"
    );

    // documentSymbol lists the module and the func.
    let id = lsp.request(
        "textDocument/documentSymbol",
        json!({"textDocument": {"uri": valid_uri}}),
    );
    let resp = lsp.wait_response(id);
    let symbols = resp["result"].as_array().expect("symbol array");
    assert_eq!(symbols[0]["name"], "@m");
    let names: Vec<&str> = symbols[0]["children"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["name"].as_str())
        .collect();
    assert!(names.contains(&"@foo"), "func listed: {names:?}");

    // definition on the `sum` use (line 7 `llvm.return sum`, col of "sum").
    let use_line = 7u32;
    let use_col = VALID.lines().nth(7).unwrap().find("sum").unwrap() as u32;
    let id = lsp.request(
        "textDocument/definition",
        json!({
            "textDocument": {"uri": valid_uri},
            "position": {"line": use_line, "character": use_col}
        }),
    );
    let resp = lsp.wait_response(id);
    let def = &resp["result"];
    assert_eq!(def["uri"], valid_uri);
    // def site: line 6, `sum = llvm.add ...`
    assert_eq!(def["range"]["start"]["line"], 6);
    let def_col = VALID.lines().nth(6).unwrap().find("sum").unwrap() as u64;
    assert_eq!(def["range"]["start"]["character"], def_col);

    // (b) broken IR: at least one diagnostic with a sane range.
    let broken_uri = "file:///test/broken.plir";
    lsp.notify(
        "textDocument/didOpen",
        json!({"textDocument": {
            "uri": broken_uri, "languageId": "plir", "version": 1, "text": BROKEN}}),
    );
    let diags = lsp.wait_diagnostics(broken_uri);
    let diags = diags.as_array().expect("array");
    assert!(!diags.is_empty(), "broken IR must produce diagnostics");
    let d = &diags[0];
    let line = d["range"]["start"]["line"].as_u64().unwrap();
    assert!(
        (1..BROKEN.lines().count() as u64).contains(&line),
        "diagnostic points into the document: {d}"
    );
    assert!(
        d["message"].as_str().unwrap_or("").len() > 5,
        "diagnostic has a message: {d}"
    );

    lsp.shutdown();
}
