use biwac_span::Span;

use crate::{ContractTy, Entrypoint};

#[derive(Debug)]
pub enum EntrypointError {
    /// エントリポイントのシグネチャが期待と違う。
    ///
    /// `main` は `()` である (期待は表 `Entrypoint` が持つ)。
    InvalidSignature {
        name: String,
        expected_args: &'static [ContractTy],
        expected_ret: ContractTy,
        reason: SignatureProblem,
        span: Span,
    },

    /// playable package に必須のエントリポイントが無い。
    MissingEntrypoint { entrypoint: Entrypoint },

    /// 名前は使われているが、期待した種別 (scene / 関数) ではない。
    /// 例: 以前のエントリポイントの形 `scene main`。
    EntrypointWrongKind { entrypoint: Entrypoint, span: Span },
}

#[derive(Debug)]
pub enum SignatureProblem {
    /// 引数の個数が期待と違う。
    ArgCount { found: usize, expected: usize },

    /// `index` 番目 (0 始まり) の引数の型が期待と違う。
    ArgType { index: usize, expected: ContractTy },

    /// 戻り値の型が期待と違う。
    ReturnType { expected: ContractTy },

    /// レシーバ (`self`) を取っている。
    HasReceiver,
}

impl EntrypointError {
    pub fn span(&self) -> Option<&Span> {
        match self {
            Self::InvalidSignature { span, .. } | Self::EntrypointWrongKind { span, .. } => {
                Some(span)
            }
            Self::MissingEntrypoint { .. } => None,
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::InvalidSignature {
                name,
                expected_args,
                expected_ret,
                reason,
                ..
            } => {
                let detail = match reason {
                    SignatureProblem::ArgCount { found, expected } => {
                        format!("it takes {found} argument(s) instead of {expected}")
                    }
                    SignatureProblem::ArgType { index, expected } => format!(
                        "its argument #{} is not a {}",
                        index + 1,
                        expected.describe()
                    ),
                    SignatureProblem::ReturnType { expected } => match expected {
                        ContractTy::Void => "it returns a value".to_string(),
                    },
                    SignatureProblem::HasReceiver => "it takes a receiver".to_string(),
                };

                let args = if expected_args.is_empty() {
                    "no argument".to_string()
                } else {
                    let list: Vec<&str> = expected_args.iter().map(|a| a.describe()).collect();
                    format!("exactly ({})", list.join(", "))
                };
                let ret = match expected_ret {
                    ContractTy::Void => "return nothing",
                };
                let expected = format!("take {args} and {ret}");

                format!("`{name}` must {expected}, but {detail}")
            }
            Self::MissingEntrypoint { entrypoint } => format!(
                "this playable package must define {} `{}` in the root module",
                entrypoint.kind().describe(),
                entrypoint.name()
            ),
            Self::EntrypointWrongKind { entrypoint, .. } => {
                let hint = match entrypoint {
                    // 以前は `scene main` がエントリポイントだった。
                    Entrypoint::Main => {
                        " (a scene is no longer an entry point: pass it to `Window::new` as \
                         the `main_scene` and call `show()` in `fn main()`)"
                    }
                };
                format!(
                    "the runtime calls `{}` directly, so it must be {}{}",
                    entrypoint.name(),
                    entrypoint.kind().describe(),
                    hint
                )
            }
        }
    }
}

impl biwac_base::BiwacError for EntrypointError {
    fn print_error_message(&self, _ctx: &biwac_base::ErrorContext) {
        // TODO: 他のエラー系 crate と同様、ariadne による
        //       ソース抜粋付きの診断表示は未実装。
        eprintln!("Error: {}", self.message());
    }
}
