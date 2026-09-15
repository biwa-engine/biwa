use biwa_lsp_lexer::SyntaxKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BiwaLanguage {}

impl rowan::Language for BiwaLanguage {
    type Kind = SyntaxKind;

    fn kind_from_raw(raw: rowan::SyntaxKind) -> SyntaxKind {
        // SAFETY: SyntaxKind は連続した u16 値で定義されている
        assert!(raw.0 <= SyntaxKind::ParenExpr as u16);
        unsafe { std::mem::transmute::<u16, SyntaxKind>(raw.0) }
    }

    fn kind_to_raw(kind: SyntaxKind) -> rowan::SyntaxKind {
        kind.into()
    }
}

pub type SyntaxNode = rowan::SyntaxNode<BiwaLanguage>;
pub type SyntaxToken = rowan::SyntaxToken<BiwaLanguage>;
pub type SyntaxElement = rowan::SyntaxElement<BiwaLanguage>;
