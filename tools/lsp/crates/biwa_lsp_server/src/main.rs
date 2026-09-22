mod semantic_tokens;

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

use biwa_lsp_highlight::{HighlightToken, TokenType, highlight};
use semantic_tokens::{
    TOKEN_TYPES_LEGEND, build_line_index, encode_semantic_tokens, offset_to_line_col,
};

struct Backend {
    client: Client,
    // ファイル URI → テキスト内容
    documents: Arc<RwLock<HashMap<Url, String>>>,
    // ファイル URI → 直近の名前解決結果 (publish_diagnostics で更新し、
    // semantic_tokens_full が読み直す。パッケージ全体のロード・解決は
    // 決して安くないので、リクエストのたびに 2 回走らせない)。
    resolutions: Arc<RwLock<HashMap<Url, biwa_lsp_resolve::DocumentResolution>>>,
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _params: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: "biwa-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            capabilities: ServerCapabilities {
                // テキスト変更をフルで受け取る
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            legend: SemanticTokensLegend {
                                token_types: TOKEN_TYPES_LEGEND
                                    .iter()
                                    .map(|s| SemanticTokenType::new(s))
                                    .collect(),
                                token_modifiers: vec![],
                            },
                            full: Some(SemanticTokensFullOptions::Bool(true)),
                            range: None,
                            work_done_progress_options: Default::default(),
                        },
                    ),
                ),
                ..Default::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        // 「今動いているバイナリはどれか」をすぐ確認できるように、実行ファイルの
        // パスと更新時刻を出す (再ビルド後に古いバイナリを掴んでいないかの確認用)。
        let exe_info = std::env::current_exe()
            .ok()
            .map(|p| {
                let modified = std::fs::metadata(&p)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .map(|t| format!("{t:?}"))
                    .unwrap_or_else(|| "unknown".to_string());
                format!("{} (modified: {modified})", p.display())
            })
            .unwrap_or_else(|| "unknown".to_string());

        self.client
            .log_message(
                MessageType::INFO,
                format!(
                    "biwa-lsp v{} initialized, binary: {exe_info}",
                    env!("CARGO_PKG_VERSION")
                ),
            )
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let text = params.text_document.text;
        self.documents
            .write()
            .await
            .insert(uri.clone(), text.clone());
        self.publish_diagnostics(&uri, &text).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        // FULL sync なので常に最後の change がドキュメント全体
        if let Some(change) = params.content_changes.into_iter().last() {
            let text = change.text;
            self.documents
                .write()
                .await
                .insert(uri.clone(), text.clone());
            self.publish_diagnostics(&uri, &text).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.documents
            .write()
            .await
            .remove(&params.text_document.uri);
        self.resolutions
            .write()
            .await
            .remove(&params.text_document.uri);
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let uri = &params.text_document.uri;
        let docs = self.documents.read().await;
        let Some(src) = docs.get(uri) else {
            self.client
                .log_message(
                    MessageType::WARNING,
                    format!("semanticTokens/full: {uri} is not an open document (no cached text)"),
                )
                .await;
            return Ok(None);
        };

        let mut highlight_tokens = highlight(src);
        if let Some(resolution) = self.resolutions.read().await.get(uri) {
            apply_resolved_classifications(&mut highlight_tokens, &resolution.classifications);
        }
        let data = encode_semantic_tokens(src, &highlight_tokens);
        self.client
            .log_message(
                MessageType::LOG,
                format!(
                    "semanticTokens/full: {uri} -> {} highlight tokens, {} encoded",
                    highlight_tokens.len(),
                    data.len()
                ),
            )
            .await;

        Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
            result_id: None,
            data,
        })))
    }
}

impl Backend {
    /// パースエラーと名前解決エラーを合わせて Diagnostics として publish する。
    async fn publish_diagnostics(&self, uri: &Url, src: &str) {
        let parse_result = biwa_lsp_highlight::parse_for_diagnostics(src);
        let line_starts = build_line_index(src);
        let mut diagnostics: Vec<Diagnostic> = parse_result
            .errors
            .iter()
            .map(|e| to_lsp_diagnostic(&line_starts, src, e.start, e.end, &e.message))
            .collect();

        // 名前解決はパッケージ全体を読み直す重い処理で、`biwa-package.json` が
        // 見つからない場合など走らせようがないこともある。そうした場合は
        // 構文だけの診断・ハイライトに静かにフォールバックする。
        match uri.to_file_path() {
            Ok(path) => {
                match biwa_lsp_resolve::resolve_document(&path, src) {
                    Ok(resolution) => {
                        diagnostics.extend(resolution.diagnostics.iter().map(|d| {
                            to_lsp_diagnostic(&line_starts, src, d.start, d.end, &d.message)
                        }));
                        self.resolutions
                            .write()
                            .await
                            .insert(uri.clone(), resolution);
                    }
                    Err(reason) => {
                        self.resolutions.write().await.remove(uri);
                        self.client
                            .log_message(
                                MessageType::INFO,
                                format!("name resolution skipped for {uri}: {reason}"),
                            )
                            .await;
                    }
                }
            }
            Err(()) => {
                self.resolutions.write().await.remove(uri);
            }
        }

        self.client
            .log_message(
                MessageType::INFO,
                format!(
                    "publishDiagnostics: {uri} -> {} error(s)",
                    diagnostics.len()
                ),
            )
            .await;

        self.client
            .publish_diagnostics(uri.clone(), diagnostics, None)
            .await;
    }
}

fn to_lsp_diagnostic(
    line_starts: &[usize],
    src: &str,
    start: usize,
    end: usize,
    message: &str,
) -> Diagnostic {
    let (start_line, start_col) = offset_to_line_col(line_starts, src, start);
    let (end_line, end_col) = offset_to_line_col(line_starts, src, end);
    Diagnostic {
        range: Range {
            start: Position::new(start_line, start_col),
            end: Position::new(end_line, end_col),
        },
        severity: Some(DiagnosticSeverity::ERROR),
        message: message.to_string(),
        source: Some("biwa-lsp".to_string()),
        ..Default::default()
    }
}

/// 名前解決による識別子の分類で、構文だけで決めた token_type を上書きする。
/// span が完全一致するものだけを対象にする
/// (`biwa_lsp_resolve::Classification` は識別子 1 つぶんの span しか出さない)。
fn apply_resolved_classifications(
    tokens: &mut [HighlightToken],
    classifications: &[biwa_lsp_resolve::Classification],
) {
    use std::collections::HashMap;

    let overrides: HashMap<(usize, usize), TokenType> = classifications
        .iter()
        .map(|c| ((c.start, c.end), resolved_kind_to_token_type(c.kind)))
        .collect();

    for tok in tokens.iter_mut() {
        if let Some(tt) = overrides.get(&(tok.start, tok.end)) {
            tok.token_type = *tt;
        }
    }
}

fn resolved_kind_to_token_type(kind: biwa_lsp_resolve::ResolvedKind) -> TokenType {
    match kind {
        biwa_lsp_resolve::ResolvedKind::Function => TokenType::Function,
        biwa_lsp_resolve::ResolvedKind::Parameter => TokenType::Parameter,
        biwa_lsp_resolve::ResolvedKind::Type => TokenType::Type,
        biwa_lsp_resolve::ResolvedKind::Interface => TokenType::Interface,
        biwa_lsp_resolve::ResolvedKind::Namespace => TokenType::Namespace,
    }
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| Backend {
        client,
        documents: Arc::new(RwLock::new(HashMap::new())),
        resolutions: Arc::new(RwLock::new(HashMap::new())),
    });

    Server::new(stdin, stdout, socket).serve(service).await;
}
