use biwac_base::{BiwacError, DiagSpan, ModPath};
use biwac_span::Span;

use colored::Colorize;

#[derive(Debug)]
pub enum PkgLoadError<'src> {
    RootModuleDuplicated,
    RootModuleNotFound,
    /// 字句解析・構文解析いずれかの失敗。`SourceParser::parse` はどちらの
    /// 段で失敗したかを区別せず `Box<dyn BiwacError>` に包んで返すので、
    /// ここでも 1 種類にまとめている (`print_error_message` に委譲するだけで
    /// 種別を見る場所はどこにも無い)。
    ParseError {
        modpath: ModPath,
        err: Box<dyn BiwacError + 'src>,
    },
    /// `mod foo;` と宣言されているのに、`foo.biwa` が無い。
    ModuleFileNotFound {
        name: String,
        /// 期待したファイル (`src/` からの相対パス)。
        expected: String,
        span: Span,
    },
    /// `.biwa` ファイルがあるのに、どの `mod` 宣言からも届かない。
    UndeclaredModuleFile {
        /// そのファイル (`src/` からの相対パス)。
        path: String,
        /// 宣言を書くべきモジュールのファイル。
        /// 対応するモジュールファイルの無いディレクトリの中にあるなら `None`。
        declare_in: Option<String>,
    },
    /// 同じ名前の `mod` 宣言が 2 つある。
    DuplicatedModDecl {
        name: String,
        first: Span,
        second: Span,
    },
    /// ルートモジュールで `mod main;` / `mod lib;` と宣言した。
    /// `src/main.biwa` / `src/lib.biwa` はルートモジュールそのものなので子にできない。
    RootModuleNameDeclared {
        name: String,
        span: Span,
    },
}

fn at(span: &Span) -> DiagSpan {
    DiagSpan::new(span.module(), span.begin(), span.end())
}

impl BiwacError for PkgLoadError<'_> {
    fn print_error_message(&self, ctx: &biwac_base::ErrorContext) {
        match self {
            Self::RootModuleDuplicated => {
                println!(
                    r#"{} Root module duplicated.
Both of `{}.{}` or `{}.{}` exist in a package.
Only one of them can exist."#,
                    "Error:".red(),
                    biwac_base::BIWA_LIBRARY_PACKAGE_ROOT_MODULE_NAME,
                    biwac_base::BIWA_EXTENSION,
                    biwac_base::BIWA_BINARY_PACKAGE_ROOT_MODULE_NAME,
                    biwac_base::BIWA_EXTENSION,
                )
            }
            Self::RootModuleNotFound => {
                println!(
                    r#"{} Root module not found.
One of `{}.{}` or `{}.{}` needed in a package."#,
                    "Error:".red(),
                    biwac_base::BIWA_LIBRARY_PACKAGE_ROOT_MODULE_NAME,
                    biwac_base::BIWA_EXTENSION,
                    biwac_base::BIWA_BINARY_PACKAGE_ROOT_MODULE_NAME,
                    biwac_base::BIWA_EXTENSION,
                )
            }
            Self::ParseError { err, .. } => {
                err.print_error_message(ctx);
            }
            Self::ModuleFileNotFound {
                name,
                expected,
                span,
            } => {
                ctx.diagnostic(format!("File for module `{name}` not found."))
                    .label(at(span), format!("`{expected}` is expected"))
                    .print();
            }
            Self::UndeclaredModuleFile { path, declare_in } => {
                let d = ctx.diagnostic(format!("`{path}` is not declared as a module."));
                match declare_in {
                    Some(parent) => d.note(format!(
                        "declare it with `mod <name>;` in `{parent}`, or remove the file"
                    )),
                    None => d.note(
                        "its directory has no module file of the same name, so no module can declare it; \
                         add the module file and declare it with `mod <name>;`, or remove the file",
                    ),
                }
                .print();
            }
            Self::DuplicatedModDecl {
                name,
                first,
                second,
            } => {
                ctx.diagnostic(format!("Module `{name}` is declared twice."))
                    .label(at(second), "declared again here")
                    .sub_label(at(first), "first declared here")
                    .print();
            }
            Self::RootModuleNameDeclared { name, span } => {
                ctx.diagnostic(format!(
                    "Module `{name}` cannot be declared in the root module."
                ))
                .label(
                    at(span),
                    format!(
                        "`src/{name}.{}` is the root module itself",
                        biwac_base::BIWA_EXTENSION
                    ),
                )
                .print();
            }
        }
    }
}
