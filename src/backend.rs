use crate::config::Config;
use log::debug;
use std::collections::HashMap;
use std::default::Default;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use sv_parser::{parse_sv_str, Define, DefineText};
use svlint::config::Config as LintConfig;
use svlint::linter::Linter;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{async_trait, Client, LanguageServer};

pub struct Backend {
    client: Client,
    root_uri: Arc<RwLock<Option<Url>>>,
    config: Arc<RwLock<Option<Config>>>,
    linter: Arc<RwLock<Option<Linter>>>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Backend {
            client,
            root_uri: Default::default(),
            config: Default::default(),
            linter: Default::default(),
        }
    }

    fn lint(&self, s: &str) -> Vec<Diagnostic> {
        let mut ret = Vec::new();

        let root_uri = self.root_uri.read().unwrap();
        let root_uri = if let Some(ref root_uri) = *root_uri {
            if let Ok(root_uri) = root_uri.to_file_path() {
                root_uri
            } else {
                PathBuf::from("")
            }
        } else {
            PathBuf::from("")
        };

        let config = self.config.read().unwrap();
        let mut include_paths = Vec::new();
        let mut defines = HashMap::new();
        if let Some(ref config) = *config {
            for path in &config.verilog.include_paths {
                let mut p = root_uri.clone();
                p.push(PathBuf::from(path));
                include_paths.push(p);
            }
            for define in &config.verilog.defines {
                let mut define = define.splitn(2, '=');
                let ident = String::from(define.next().unwrap());
                let text = if let Some(x) = define.next() {
                    if let Ok(x) = enquote::unescape(x, None) {
                        Some(DefineText::new(x, None))
                    } else {
                        None
                    }
                } else {
                    None
                };
                let define = Define::new(ident.clone(), vec![], text);
                defines.insert(ident, Some(define));
            }
        };
        debug!("include_paths: {:?}", include_paths);
        debug!("defines: {:?}", defines);

        let parsed = parse_sv_str(
            s,
            &PathBuf::from(""),
            &defines,
            &include_paths,
            false,
            false,
        );
        match parsed {
            Ok((syntax_tree, _new_defines)) => {
                let mut linter = self.linter.write().unwrap();
                if let Some(ref mut linter) = *linter {
                    for event in syntax_tree.into_iter().event() {
                        for failed in linter.check(&syntax_tree, &event) {
                            debug!("{:?}", failed);
                            if failed.path != PathBuf::from("") {
                                continue;
                            }
                            let (line, col) = get_position(s, failed.beg);
                            ret.push(Diagnostic::new(
                                Range::new(
                                    Position::new(line, col),
                                    Position::new(line, col + failed.len as u32),
                                ),
                                Some(DiagnosticSeverity::Warning),
                                Some(NumberOrString::String(failed.name)),
                                Some(String::from("svls")),
                                failed.hint,
                                None,
                                None,
                            ));
                        }
                    }
                }
            }
            Err(x) => {
                debug!("parse_error: {:?}", x);
                if let sv_parser::Error::Parse(Some((path, pos))) = x {
                    if path == PathBuf::from("") {
                        let (line, col) = get_position(s, pos);
                        let line_end = get_line_end(s, pos);
                        let len = line_end - pos as u32;
                        ret.push(Diagnostic::new(
                            Range::new(Position::new(line, col), Position::new(line, col + len)),
                            Some(DiagnosticSeverity::Error),
                            None,
                            Some(String::from("svls")),
                            String::from("parse error"),
                            None,
                            None,
                        ));
                    }
                }
            }
        }
        ret
    }
}

#[async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        debug!("root_uri: {:?}", params.root_uri);

        let config_svls = search_config(&PathBuf::from(".svls.toml"));
        debug!("config_svls: {:?}", config_svls);
        let config = match generate_config(config_svls) {
            Ok(x) => x,
            Err(x) => {
                self.client.show_message(MessageType::Warning, &x).await;
                Config::default()
            }
        };

        if config.option.linter {
            let config_svlint = search_config(&PathBuf::from(".svlint.toml"));
            debug!("config_svlint: {:?}", config_svlint);

            let linter = match generate_linter(config_svlint) {
                Ok(x) => x,
                Err(x) => {
                    self.client.show_message(MessageType::Warning, &x).await;
                    Linter::new(LintConfig::new().enable_all())
                }
            };

            let mut w = self.linter.write().unwrap();
            *w = Some(linter);
        }

        let mut w = self.root_uri.write().unwrap();
        *w = params.root_uri.clone();

        let mut w = self.config.write().unwrap();
        *w = Some(config);

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::Full,
                )),
                workspace: Some(WorkspaceServerCapabilities {
                    workspace_folders: Some(WorkspaceFoldersServerCapabilities {
                        supported: Some(true),
                        change_notifications: Some(OneOf::Left(true)),
                    }),
                    file_operations: None,
                }),
                ..ServerCapabilities::default()
            },
            server_info: Some(ServerInfo {
                name: String::from("svls"),
                version: Some(String::from(env!("CARGO_PKG_VERSION"))),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::Info, &"server initialized".to_string())
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_change_workspace_folders(&self, _: DidChangeWorkspaceFoldersParams) {}

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        debug!("did_open");
        let diag = self.lint(&params.text_document.text);
        self.client
            .publish_diagnostics(
                params.text_document.uri,
                diag,
                Some(params.text_document.version),
            )
            .await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        debug!("did_change");
        let diag = self.lint(&params.content_changes[0].text);
        self.client
            .publish_diagnostics(
                params.text_document.uri,
                diag,
                Some(params.text_document.version),
            )
            .await;
    }
}

fn search_config(config: &Path) -> Option<PathBuf> {
    if let Ok(current) = env::current_dir() {
        for dir in current.ancestors() {
            let candidate = dir.join(config);
            if candidate.exists() {
                return Some(candidate);
            }
        }
        None
    } else {
        None
    }
}

fn generate_config(config: Option<PathBuf>) -> std::result::Result<Config, String> {
    if let Some(config) = config {
        if let Ok(s) = std::fs::read_to_string(&config) {
            if let Ok(config) = toml::from_str(&s) {
                Ok(config)
            } else {
                Err(format!(
                    "Failed to parse {}. Enable all lint rules.",
                    config.to_string_lossy()
                ))
            }
        } else {
            Err(format!(
                "Failed to read {}. Enable all lint rules.",
                config.to_string_lossy()
            ))
        }
    } else {
        Ok(Config::default())
    }
}

fn generate_linter(config: Option<PathBuf>) -> std::result::Result<Linter, String> {
    if let Some(config) = config {
        if let Ok(s) = std::fs::read_to_string(&config) {
            if let Ok(config) = toml::from_str(&s) {
                Ok(Linter::new(config))
            } else {
                Err(format!(
                    "Failed to parse {}. Enable all lint rules.",
                    config.to_string_lossy()
                ))
            }
        } else {
            Err(format!(
                "Failed to read {}. Enable all lint rules.",
                config.to_string_lossy()
            ))
        }
    } else {
        Err(".svlint.toml is not found. Enable all lint rules.".to_string())
    }
}

fn get_position(s: &str, pos: usize) -> (u32, u32) {
    let mut line = 0;
    let mut col = 0;
    let mut p = 0;
    while p < pos {
        if let Some(c) = s.get(p..p + 1) {
            if c == "\n" {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
        } else {
            col += 1;
        }
        p += 1;
    }
    (line, col)
}

fn get_line_end(s: &str, pos: usize) -> u32 {
    let mut p = pos;
    while p < s.len() {
        if let Some(c) = s.get(p..p + 1) {
            if c == "\n" {
                break;
            }
        }
        p += 1;
    }
    p as u32
}
