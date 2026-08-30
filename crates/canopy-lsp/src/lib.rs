//! A language server reporting what the linter finds in a workflow.
//!
//! Holds nothing but the text of the open documents: every change re-reads and re-lints the
//! whole file, which is quick enough for something a person is typing into.

mod github;
mod symbols;
mod uses;

use std::collections::HashMap;
use std::sync::Mutex;

use gh_actions_lint::Severity;
use gh_actions_spec::Workflow;
use tower_lsp_server::lsp_types::*;
use tower_lsp_server::{Client, LanguageServer, LspService, Server, jsonrpc};
use yaml_with_spans::{Document, Span};

use github::Refs;

#[tokio::main]
pub async fn run() {
    let (service, socket) = LspService::new(Backend::new);
    Server::new(tokio::io::stdin(), tokio::io::stdout(), socket)
        .serve(service)
        .await;
}

struct Backend {
    client: Client,
    documents: Mutex<HashMap<Uri, String>>,
    refs: Refs,
}

impl Backend {
    fn new(client: Client) -> Self {
        Self {
            client,
            documents: Mutex::new(HashMap::new()),
            refs: Refs::new(),
        }
    }

    async fn update(&self, uri: Uri, text: String) {
        let diagnostics = analyze(&text);
        self.documents.lock().unwrap().insert(uri.clone(), text);
        self.client
            .publish_diagnostics(uri, diagnostics, None)
            .await;
    }

    fn text_of(&self, uri: &Uri) -> Option<String> {
        self.documents.lock().unwrap().get(uri).cloned()
    }
}

impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> jsonrpc::Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                // Whole documents rather than incremental edits: linting re-reads anyway.
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                inlay_hint_provider: Some(OneOf::Left(true)),
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
                document_symbol_provider: Some(OneOf::Left(true)),
                code_lens_provider: Some(CodeLensOptions {
                    resolve_provider: Some(false),
                }),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "canopy-lsp".to_string(),
                version: None,
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "canopy-lsp ready")
            .await;
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let document = params.text_document;
        self.update(document.uri, document.text).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        // Full sync: the final change carries the whole document.
        if let Some(change) = params.content_changes.into_iter().next_back() {
            self.update(params.text_document.uri, change.text).await;
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let uri = params.text_document.uri;
        if let Some(text) = self.text_of(&uri) {
            self.update(uri, text).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        self.documents.lock().unwrap().remove(&uri);
        // Clears what was reported, so a closed file leaves nothing behind.
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }

    /// The second is what makes pinning bearable. A pinned workflow is forty characters of
    /// hex per action and no way to tell `v4` from `v2` by reading it; showing the tag gives
    /// that back without writing anything into the file.
    async fn inlay_hint(&self, params: InlayHintParams) -> jsonrpc::Result<Option<Vec<InlayHint>>> {
        let Some(text) = self.text_of(&params.text_document.uri) else {
            return Ok(None);
        };
        let Ok(document) = Document::parse(&text) else {
            return Ok(None);
        };

        let mut hints = Vec::new();
        for reference in uses::references(&document, &text) {
            if !within(&params.range, reference.span.start.line) {
                continue;
            }
            let (owner, repo, at) = (&reference.owner, &reference.repo, &reference.at);

            let label = if github::is_commit(at) {
                self.refs.tag(owner, repo, at).await
            } else {
                self.refs
                    .published(owner, repo, at)
                    .await
                    .map(|day| format!("published {day}"))
            };
            let Some(label) = label else { continue };

            hints.push(InlayHint {
                position: to_position(reference.span.end, &text),
                label: InlayHintLabel::String(label),
                kind: None,
                text_edits: None,
                tooltip: None,
                padding_left: Some(true),
                padding_right: None,
                data: None,
            });
        }

        Ok(Some(hints))
    }

    /// Nothing is written into a comment to remember what the reference was, the way other
    /// tools do, because the hint above already shows it.
    async fn code_action(
        &self,
        params: CodeActionParams,
    ) -> jsonrpc::Result<Option<CodeActionResponse>> {
        let uri = params.text_document.uri;
        let Some(text) = self.text_of(&uri) else {
            return Ok(None);
        };
        let Ok(document) = Document::parse(&text) else {
            return Ok(None);
        };

        let mut actions = Vec::new();
        for reference in uses::references(&document, &text) {
            if !within(&params.range, reference.span.start.line) {
                continue;
            }
            let (owner, repo, at) = (&reference.owner, &reference.repo, &reference.at);
            // Already pinned: there is nothing to offer.
            if github::is_commit(at) {
                continue;
            }
            let Some(commit) = self.refs.commit(owner, repo, at).await else {
                continue;
            };

            let edit = TextEdit {
                range: to_range(reference.at_span, &text),
                new_text: commit.clone(),
            };
            actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: format!("Pin {at} to {}", &commit[..commit.len().min(12)]),
                kind: Some(CodeActionKind::QUICKFIX),
                edit: Some(WorkspaceEdit {
                    changes: Some(HashMap::from([(uri.clone(), vec![edit])])),
                    ..Default::default()
                }),
                ..Default::default()
            }));
        }

        Ok(Some(actions))
    }

    /// A standard request rather than something of our own, so an editor gets the outline
    /// and the breadcrumbs for free, and so anything that speaks LSP can list the jobs.
    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> jsonrpc::Result<Option<DocumentSymbolResponse>> {
        let Some(text) = self.text_of(&params.text_document.uri) else {
            return Ok(None);
        };
        let Ok(document) = Document::parse(&text) else {
            return Ok(None);
        };

        let found = symbols::jobs(&document)
            .into_iter()
            .map(|job| symbol(&job, &text))
            .collect();

        Ok(Some(DocumentSymbolResponse::Nested(found)))
    }

    /// The lens only says what could be run and where; running it is for the editor, which
    /// is better placed to own something long-lived than a language server is.
    async fn code_lens(&self, params: CodeLensParams) -> jsonrpc::Result<Option<Vec<CodeLens>>> {
        let uri = params.text_document.uri;
        let Some(text) = self.text_of(&uri) else {
            return Ok(None);
        };
        let Ok(document) = Document::parse(&text) else {
            return Ok(None);
        };

        let mut lenses = Vec::new();
        for job in symbols::jobs(&document) {
            lenses.push(CodeLens {
                range: to_range(job.key, &text),
                command: Some(Command {
                    title: "▶ Run job".to_string(),
                    command: RUN_COMMAND.to_string(),
                    arguments: Some(vec![
                        serde_json::Value::String(uri.to_string()),
                        serde_json::Value::String(job.id.to_string()),
                    ]),
                }),
                data: None,
            });
        }

        if let Some(jobs) = document
            .root
            .as_mapping()
            .and_then(|root| root.entry("jobs"))
        {
            lenses.push(CodeLens {
                range: to_range(jobs.key.span, &text),
                command: Some(Command {
                    title: "▶ Run workflow".to_string(),
                    command: RUN_COMMAND.to_string(),
                    arguments: Some(vec![serde_json::Value::String(uri.to_string())]),
                }),
                data: None,
            });
        }

        Ok(Some(lenses))
    }

    async fn shutdown(&self) -> jsonrpc::Result<()> {
        Ok(())
    }
}

/// Named rather than invoked: the server does not run workflows, it only says what could be.
const RUN_COMMAND: &str = "canopy.run";

fn symbol(job: &symbols::Job<'_>, text: &str) -> DocumentSymbol {
    let steps = job
        .steps()
        .into_iter()
        .map(|step| {
            let range = to_range(step.span(), text);
            #[allow(deprecated)]
            DocumentSymbol {
                name: step.name().to_string(),
                detail: None,
                kind: SymbolKind::METHOD,
                tags: None,
                deprecated: None,
                range,
                selection_range: range,
                children: None,
            }
        })
        .collect();

    #[allow(deprecated)]
    DocumentSymbol {
        name: job.id.to_string(),
        detail: job.name().map(str::to_string),
        kind: SymbolKind::FUNCTION,
        tags: None,
        deprecated: None,
        range: to_range(job.whole, text),
        // Selecting the symbol puts the cursor on the key, not the whole block.
        selection_range: to_range(job.key, text),
        children: Some(steps),
    }
}

fn within(range: &Range, line: u32) -> bool {
    (range.start.line..=range.end.line).contains(&line)
}

fn analyze(text: &str) -> Vec<Diagnostic> {
    // The document is read once and kept: the rules work on the workflow it builds, and
    // the spans it remembers are what turn a finding into something the editor can draw.
    let document = match Document::parse(text) {
        Ok(document) => document,
        Err(err) => return vec![parse_error(&err, text)],
    };
    let workflow: Workflow = match yaml_with_spans::from_node(&document.root) {
        Ok(workflow) => workflow,
        Err(err) => return vec![parse_error(&err, text)],
    };

    // Validation first: what would stop this workflow running is not an opinion, and no
    // `canopy:ignore` comment applies to it. Then the warnings, which are and do.
    let mut findings = gh_actions_plan::validate::check(&workflow);
    findings.extend(gh_actions_lint::check(&document, &workflow));

    findings
        .iter()
        .map(|finding| to_lsp(finding, &document, text))
        .collect()
}

fn to_lsp(finding: &gh_actions_lint::Diagnostic, document: &Document, text: &str) -> Diagnostic {
    Diagnostic {
        range: locate(document, &finding.location, text),
        severity: Some(match finding.severity {
            Severity::Error => DiagnosticSeverity::ERROR,
            Severity::Warning => DiagnosticSeverity::WARNING,
        }),
        source: Some("canopy".to_string()),
        code: Some(NumberOrString::String(finding.rule.to_string())),
        message: finding.message.clone(),
        ..Default::default()
    }
}

/// A rule may point at something that is not there — a missing field is a common thing to
/// complain about — so the path is shortened a segment at a time until it names something,
/// which lands the finding on the nearest thing that does exist.
fn locate(document: &Document, path: &str, text: &str) -> Range {
    document
        .locate(path)
        .map_or_else(Range::default, |node| to_range(node.span, text))
}

fn parse_error(err: &yaml_with_spans::Error, text: &str) -> Diagnostic {
    let at = to_position(err.position, text);

    Diagnostic {
        range: Range::new(at, Position::new(at.line, at.character + 1)),
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("canopy".to_string()),
        message: err.message.clone(),
        ..Default::default()
    }
}

fn to_range(span: Span, text: &str) -> Range {
    Range::new(to_position(span.start, text), to_position(span.end, text))
}

/// Converts a position in bytes to one in the UTF-16 units an editor counts.
fn to_position(at: yaml_with_spans::Position, text: &str) -> Position {
    let line_start = at.offset - at.column as usize;
    let column = text
        .get(line_start..at.offset)
        .map_or(0, |prefix| prefix.encode_utf16().count());

    Position::new(at.line, column as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finding_is_not_placed_by_what_a_script_happens_to_say() {
        // The `needs:` inside the shell script is text, not a field. Reading the file
        // rather than searching it is what tells the two apart.
        let found = analyze(
            r#"name: Test
on: push
permissions: {}
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: |
          cat <<EOF
          needs: pretend
          EOF
    needs: gone
"#,
        );

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].range.start.line, 11);
    }

    #[test]
    fn a_sound_workflow_reports_nothing() {
        let found = analyze(
            r#"name: Test
on: push
permissions: {}
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo hi
"#,
        );

        assert!(found.is_empty(), "unexpected: {found:?}");
    }

    #[test]
    fn a_finding_carries_its_rule_and_its_line() {
        let found = analyze(
            r#"name: Test
on: push
permissions: {}
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - name: nothing
"#,
        );

        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].code,
            Some(NumberOrString::String("step-shape".to_string()))
        );
        assert_eq!(found[0].severity, Some(DiagnosticSeverity::ERROR));
        assert_eq!(found[0].range.start.line, 7);
    }

    #[test]
    fn a_file_that_will_not_parse_says_so_once() {
        let found = analyze("jobs:\n  build:\n  - not a job\n");

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].source.as_deref(), Some("canopy"));
    }
}
