use biwac_span::Span;

#[derive(Debug)]
pub enum SceneError {
    /// scene のシグネチャが `(Game[..]) -> Game[..]` でない。
    InvalidSignature {
        name: String,
        reason: SignatureProblem,
        span: Span,
    },
}

#[derive(Debug)]
pub enum SignatureProblem {
    /// 引数の個数が 1 でない。
    ArgCount { found: usize },

    /// `index` 番目 (0 始まり) の引数の型が `Game` ではない。
    ArgType { index: usize },

    /// 戻り値の型が `Game` ではない。
    ReturnType,

    /// レシーバ (`self`) を取っている。
    HasReceiver,
}

impl SceneError {
    pub fn span(&self) -> Option<&Span> {
        match self {
            Self::InvalidSignature { span, .. } => Some(span),
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::InvalidSignature { name, reason, .. } => {
                let detail = match reason {
                    SignatureProblem::ArgCount { found } => {
                        format!("it takes {found} argument(s) instead of 1")
                    }
                    SignatureProblem::ArgType { index } => {
                        format!("its argument #{} is not a `Game`", index + 1)
                    }
                    SignatureProblem::ReturnType => "it does not return a `Game`".to_string(),
                    SignatureProblem::HasReceiver => "it takes a receiver".to_string(),
                };
                format!("`{name}` must take exactly (`Game`) and return a `Game`, but {detail}")
            }
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
