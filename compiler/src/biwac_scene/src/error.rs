use biwac_span::Span;

use crate::{ContractTy, WellKnownSymbol};

#[derive(Debug)]
pub enum SceneError {
    /// ランタイムが呼ぶシンボルのシグネチャが期待と違う。
    ///
    /// scene は `(Game[..]) -> Game[..]`、`on_new_game` は
    /// `(GameWindow) -> Game[..]` である (期待は表 `WellKnownSymbol` が持つ)。
    InvalidSceneSignature {
        scene: String,
        expected_args: &'static [ContractTy],
        expected_ret: ContractTy,
        reason: SignatureProblem,
        span: Span,
    },

    /// playable package に必須のシンボルが無い。
    MissingEntryPoint { scene: WellKnownSymbol },

    /// 名前は使われているが、期待した種別 (scene / 関数) ではない。
    EntryPointNotScene { scene: WellKnownSymbol, span: Span },
}

#[derive(Debug)]
pub enum SignatureProblem {
    /// 引数の個数が期待と違う。
    ArgCount { found: usize, expected: usize },

    /// `index` 番目 (0 始まり) の引数の型が期待した lang item ではない。
    ArgType { index: usize, expected: ContractTy },

    /// 戻り値の型が期待した lang item ではない。
    ReturnType { expected: ContractTy },

    /// レシーバ (`self`) を取っている。
    HasReceiver,
}

impl SceneError {
    pub fn span(&self) -> Option<&Span> {
        match self {
            Self::InvalidSceneSignature { span, .. } | Self::EntryPointNotScene { span, .. } => {
                Some(span)
            }
            Self::MissingEntryPoint { .. } => None,
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::InvalidSceneSignature {
                scene,
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
                    SignatureProblem::ReturnType {
                        expected: ContractTy::Void,
                    } => "it returns a value".to_string(),
                    SignatureProblem::ReturnType { expected } => {
                        format!("it does not return a {}", expected.describe())
                    }
                    SignatureProblem::HasReceiver => "it takes a receiver".to_string(),
                };

                let args = if expected_args.is_empty() {
                    "no argument".to_string()
                } else {
                    let list: Vec<&str> = expected_args.iter().map(|a| a.describe()).collect();
                    format!("exactly ({})", list.join(", "))
                };
                let ret = match expected_ret {
                    ContractTy::Void => "return nothing".to_string(),
                    _ => format!("return a {}", expected_ret.describe()),
                };
                let expected = format!("take {args} and {ret}");

                format!("`{scene}` must {expected}, but {detail}")
            }
            Self::MissingEntryPoint { scene } => format!(
                "this playable package must define {} `{}` in the root module",
                scene.kind().describe(),
                scene.name()
            ),
            Self::EntryPointNotScene { scene, .. } => format!(
                "the runtime calls `{}` directly, so it must be {}",
                scene.name(),
                scene.kind().describe()
            ),
        }
    }
}

impl biwac_base::BiwacError for SceneError {
    fn print_error_message(&self, _ctx: &biwac_base::ErrorContext) {
        // TODO: 他のエラー系 crate と同様、ariadne による
        //       ソース抜粋付きの診断表示は未実装。
        eprintln!("Error: {}", self.message());
    }
}
