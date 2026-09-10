//! The synchronous LSP server loop, built on `lsp-server` + `lsp-types`
//! (rust-analyzer's stdio stack).

use std::collections::HashMap;

use anyhow::Result;
use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use lsp_types::{
    DiagnosticServerCapabilities, DocumentSymbolResponse, GotoDefinitionResponse, Hover,
    HoverContents, HoverProviderCapability, Location, MarkupContent, MarkupKind, OneOf,
    PublishDiagnosticsParams, ServerCapabilities, TextDocumentSyncCapability, TextDocumentSyncKind,
    Url,
};

use crate::analysis::DocumentAnalysis;

/// Run the LSP server over stdio until the client disconnects.
///
/// The dialects understood are exactly those linked into the calling
/// binary (they self-register on [`Context::new`](pliron::context::Context::new)).
pub fn run_stdio_server() -> Result<()> {
    let (connection, io_threads) = Connection::stdio();

    let capabilities = ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        definition_provider: Some(OneOf::Left(true)),
        references_provider: Some(OneOf::Left(true)),
        document_symbol_provider: Some(OneOf::Left(true)),
        diagnostic_provider: None::<DiagnosticServerCapabilities>,
        ..ServerCapabilities::default()
    };

    let _init_params = connection.initialize(serde_json::to_value(capabilities)?)?;
    main_loop(&connection)?;
    // Drop the connection before joining the io threads: the writer
    // thread exits only once all channel senders are gone.
    drop(connection);
    io_threads.join()?;
    Ok(())
}

fn main_loop(connection: &Connection) -> Result<()> {
    let mut docs: HashMap<Url, DocumentAnalysis> = HashMap::new();

    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    return Ok(());
                }
                let resp = handle_request(&docs, req);
                connection.sender.send(Message::Response(resp))?;
            }
            Message::Notification(not) => {
                handle_notification(connection, &mut docs, not)?;
            }
            Message::Response(_) => {}
        }
    }
    Ok(())
}

fn open_or_change(
    connection: &Connection,
    docs: &mut HashMap<Url, DocumentAnalysis>,
    uri: Url,
    text: String,
    version: Option<i32>,
) -> Result<()> {
    let analysis = DocumentAnalysis::new(text);
    let params = PublishDiagnosticsParams {
        uri: uri.clone(),
        diagnostics: analysis.diagnostics.clone(),
        version,
    };
    docs.insert(uri, analysis);
    connection.sender.send(Message::Notification(Notification {
        method: "textDocument/publishDiagnostics".into(),
        params: serde_json::to_value(params)?,
    }))?;
    Ok(())
}

fn handle_notification(
    connection: &Connection,
    docs: &mut HashMap<Url, DocumentAnalysis>,
    not: Notification,
) -> Result<()> {
    match not.method.as_str() {
        "textDocument/didOpen" => {
            let params: lsp_types::DidOpenTextDocumentParams = serde_json::from_value(not.params)?;
            open_or_change(
                connection,
                docs,
                params.text_document.uri,
                params.text_document.text,
                Some(params.text_document.version),
            )?;
        }
        "textDocument/didChange" => {
            let params: lsp_types::DidChangeTextDocumentParams =
                serde_json::from_value(not.params)?;
            // Full sync: the last change carries the whole document.
            if let Some(change) = params.content_changes.into_iter().last() {
                open_or_change(
                    connection,
                    docs,
                    params.text_document.uri,
                    change.text,
                    Some(params.text_document.version),
                )?;
            }
        }
        "textDocument/didClose" => {
            let params: lsp_types::DidCloseTextDocumentParams = serde_json::from_value(not.params)?;
            docs.remove(&params.text_document.uri);
            // Clear diagnostics for the closed document.
            let params = PublishDiagnosticsParams {
                uri: params.text_document.uri,
                diagnostics: vec![],
                version: None,
            };
            connection.sender.send(Message::Notification(Notification {
                method: "textDocument/publishDiagnostics".into(),
                params: serde_json::to_value(params)?,
            }))?;
        }
        _ => {}
    }
    Ok(())
}

fn ok_response(id: RequestId, result: impl serde::Serialize) -> Response {
    Response {
        id,
        result: Some(serde_json::to_value(result).unwrap_or(serde_json::Value::Null)),
        error: None,
    }
}

fn null_response(id: RequestId) -> Response {
    Response {
        id,
        result: Some(serde_json::Value::Null),
        error: None,
    }
}

fn handle_request(docs: &HashMap<Url, DocumentAnalysis>, req: Request) -> Response {
    let id = req.id.clone();
    match req.method.as_str() {
        "textDocument/definition" => {
            let params: lsp_types::GotoDefinitionParams = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(_) => return null_response(id),
            };
            let uri = params.text_document_position_params.text_document.uri;
            let pos = params.text_document_position_params.position;
            match docs.get(&uri).and_then(|a| a.definition(pos)) {
                Some(range) => ok_response(
                    id,
                    GotoDefinitionResponse::Scalar(Location { uri, range }),
                ),
                None => null_response(id),
            }
        }
        "textDocument/references" => {
            let params: lsp_types::ReferenceParams = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(_) => return null_response(id),
            };
            let uri = params.text_document_position.text_document.uri;
            let pos = params.text_document_position.position;
            let include_decl = params.context.include_declaration;
            match docs.get(&uri) {
                Some(a) => {
                    let locs: Vec<Location> = a
                        .references(pos, include_decl)
                        .into_iter()
                        .map(|range| Location {
                            uri: uri.clone(),
                            range,
                        })
                        .collect();
                    ok_response(id, locs)
                }
                None => null_response(id),
            }
        }
        "textDocument/hover" => {
            let params: lsp_types::HoverParams = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(_) => return null_response(id),
            };
            let uri = params.text_document_position_params.text_document.uri;
            let pos = params.text_document_position_params.position;
            match docs.get(&uri).and_then(|a| a.hover(pos)) {
                Some((range, value)) => ok_response(
                    id,
                    Hover {
                        contents: HoverContents::Markup(MarkupContent {
                            kind: MarkupKind::Markdown,
                            value,
                        }),
                        range: Some(range),
                    },
                ),
                None => null_response(id),
            }
        }
        "textDocument/documentSymbol" => {
            let params: lsp_types::DocumentSymbolParams = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(_) => return null_response(id),
            };
            match docs.get(&params.text_document.uri) {
                Some(a) => ok_response(id, DocumentSymbolResponse::Nested(a.symbols.clone())),
                None => null_response(id),
            }
        }
        _ => Response {
            id,
            result: None,
            error: Some(lsp_server::ResponseError {
                code: lsp_server::ErrorCode::MethodNotFound as i32,
                message: format!("unsupported method: {}", req.method),
                data: None,
            }),
        },
    }
}
