use ariadne::{Color, Label, Report, ReportKind, Source};

use biwac_base::{BiwacError, ModId};
use biwac_lexer::{TkKindName, Token};
use biwac_novel_parser::NovelParseError;
use biwac_span::Span;

#[derive(Debug, Clone)]
pub enum ParseError<'src> {
    InvalidToken {
        expecteds: Vec<TkKindName>,
        found: Token<'src>,
    },
    InvalidEOF {
        // EOF の場合トークンがないのでファイルを特定するために ModId を使用
        mod_id: ModId,
        expecteds: Vec<TkKindName>,
    },
    NovelParseError(NovelParseError),
    /// 書けない場所に書かれたもの (可視性・属性)。
    ///
    /// 例: enum の variant の `pub` (variant は常に enum と同じ可視性になる)。
    NotAllowedHere {
        span: Span,
        /// 書かれたもの (`"a visibility"` など)。
        what: &'static str,
        /// 書かれた場所 (`"an enum variant (..)"` など)。
        place: &'static str,
    },
}

impl BiwacError for ParseError<'_> {
    fn print_error_message(&self, ctx: &biwac_base::ErrorContext) {
        match self {
            Self::InvalidToken { expecteds, found } => {
                let modsrc = ctx.srcs.mods.get(&found.span.module()).unwrap();

                let file_name = modsrc.modu.file_name();
                let begin = modsrc.char_offset(found.span.begin());
                let end = modsrc.char_offset(found.span.end());

                Report::build(ReportKind::Error, (file_name.as_str(), begin..end))
                    .with_message("Unexpected token found.")
                    .with_label(
                        Label::new((file_name.as_str(), begin..end))
                            .with_message(if expecteds.is_empty() {
                                format!(
                                    "Another token expected, but found {}.",
                                    found.kind.pattern_with(ctx.interner)
                                )
                            } else if expecteds.len() == 1 {
                                format!(
                                    "Expected {}, but found {}.",
                                    format_token_kinds(expecteds),
                                    found.kind.pattern_with(ctx.interner)
                                )
                            } else {
                                format!(
                                    "Expected one of {}, but found {}.",
                                    format_token_kinds(expecteds),
                                    found.kind.pattern_with(ctx.interner)
                                )
                            })
                            .with_color(Color::Red),
                    )
                    .finish()
                    .print((file_name.as_str(), Source::from(&modsrc.src)))
                    .unwrap();
            }
            Self::InvalidEOF { expecteds, mod_id } => {
                let modsrc = ctx.srcs.mods.get(mod_id).unwrap();

                let file_name = modsrc.modu.file_name();
                let begin = modsrc.char_offset(modsrc.src.len());
                let end = begin;

                Report::build(ReportKind::Error, (file_name.as_str(), begin..end))
                    .with_message("Unexpected end of file found.")
                    .with_label(
                        Label::new((file_name.as_str(), begin..end))
                            .with_message(if expecteds.is_empty() {
                                "Another token expected, but found end of file.".into()
                            } else if expecteds.len() == 1 {
                                format!(
                                    "Expected {}, but found end of file.",
                                    format_token_kinds(expecteds),
                                )
                            } else {
                                format!(
                                    "Expected one of {}, but found end of file.",
                                    format_token_kinds(expecteds),
                                )
                            })
                            .with_color(Color::Red),
                    )
                    .finish()
                    .print((file_name.as_str(), Source::from(&modsrc.src)))
                    .unwrap();
            }
            Self::NovelParseError(e) => {
                e.print_error_message(ctx);
            }
            Self::NotAllowedHere { span, what, place } => {
                let modsrc = ctx.srcs.mods.get(&span.module()).unwrap();

                let file_name = modsrc.modu.file_name();
                let begin = modsrc.char_offset(span.begin());
                let end = modsrc.char_offset(span.end());

                Report::build(ReportKind::Error, (file_name.as_str(), begin..end))
                    .with_message(format!("{what} is not allowed on {place}."))
                    .with_label(
                        Label::new((file_name.as_str(), begin..end))
                            .with_message("Remove this.")
                            .with_color(Color::Red),
                    )
                    .finish()
                    .print((file_name.as_str(), Source::from(&modsrc.src)))
                    .unwrap();
            }
        }
    }
}

fn format_token_kinds(kinds: &[TkKindName]) -> String {
    if kinds.is_empty() {
        "".to_string()
    } else if kinds.len() == 1 {
        kinds[0].to_string()
    } else if kinds.len() == 2 {
        format!("{} or {}", kinds.first().unwrap(), kinds.last().unwrap())
    } else {
        format!(
            "{} or {}",
            kinds[..kinds.len() - 1]
                .iter()
                .map(|kind| kind.to_string())
                .collect::<Vec<_>>()
                .join(", "),
            kinds[kinds.len() - 1..][0]
        )
    }
}
