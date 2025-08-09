use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};
use repl_core::parse_policy;

pub struct Backend {
    client: Client,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    async fn on_change(&self, uri: Url, text: String) {
        let diagnostics = match parse_policy(&text) {
            Ok(_) => vec![],
            Err(e) => {
                // In the future, we'll create a proper span. For now, use a default range.
                let range = Range::new(Position::new(0, 0), Position::new(0, text.len() as u32));
                vec![Diagnostic::new_simple(range, e.to_string())]
            }
        };
        self.client.publish_diagnostics(uri, diagnostics, None).await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: "symbiont-repl-lsp".to_string(),
                version: Some("0.1.0".to_string()),
            }),
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                ..ServerCapabilities::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client.log_message(MessageType::INFO, "LSP server initialized.").await;
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.on_change(params.text_document.uri, params.text_document.text).await;
    }

    async fn did_change(&self, mut params: DidChangeTextDocumentParams) {
        self.on_change(params.text_document.uri, params.content_changes.remove(0).text).await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}