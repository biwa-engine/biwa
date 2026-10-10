//! CST (rowan の `SyntaxNode`) を歩くための小さな補助群。
//!
//! biwa-lsp-parser の文法は、期待した形と違うトークン列に出会っても
//! `Error` ノードとして回復しながら木を組み立て続ける (パニックモード回復)。
//! そのため lowering 側も「期待した子が無ければ `None`/`Err` を返して
//! その要素だけ諦める」という前提で書く。`unwrap` は使わない。

use biwa_lsp_lexer::SyntaxKind;
use biwa_lsp_parser::BiwaLanguage;
use biwac_base::{IdentInterner, InternedIdent, ModId};
use biwac_span::Span;

pub(crate) type SyntaxNode = rowan::SyntaxNode<BiwaLanguage>;
pub(crate) type SyntaxToken = rowan::SyntaxToken<BiwaLanguage>;
pub(crate) type SyntaxElement = rowan::SyntaxElement<BiwaLanguage>;

pub(crate) fn is_trivia(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::Whitespace | SyntaxKind::Newline | SyntaxKind::LineComment
    )
}

pub(crate) fn elem_kind(elem: &SyntaxElement) -> SyntaxKind {
    match elem {
        rowan::NodeOrToken::Node(n) => n.kind(),
        rowan::NodeOrToken::Token(t) => t.kind(),
    }
}

/// `rowan::TextRange` (u32, 文字境界の保証なし) を
/// `biwac_span::Span` (usize の byte offset) に変換する。
pub(crate) fn span_of_range(mod_id: ModId, range: rowan::TextRange) -> Span {
    Span::new(mod_id, usize::from(range.start()), usize::from(range.end()))
}

pub(crate) fn node_span(mod_id: ModId, node: &SyntaxNode) -> Span {
    span_of_range(mod_id, node.text_range())
}

pub(crate) fn token_span(mod_id: ModId, tok: &SyntaxToken) -> Span {
    span_of_range(mod_id, tok.text_range())
}

/// 識別子トークンを intern して `biwac_ast::Ident` を作る。
pub(crate) fn intern_ident_token(
    mod_id: ModId,
    interner: &mut IdentInterner,
    tok: &SyntaxToken,
) -> biwac_ast::Ident {
    let id: InternedIdent = interner.get_or_insert(tok.text());
    biwac_ast::Ident {
        id,
        span: token_span(mod_id, tok),
    }
}

/// 1 つのノードの直下の子を、trivia (空白・改行・コメント) を除いて順に取り出す cursor。
///
/// 「次はこの kind のはず」という前提で読み進める biwac_parser の
/// `TokenStream` を、トークン列ではなく CST の子要素列に対してやる版。
pub(crate) struct Children {
    items: std::vec::IntoIter<SyntaxElement>,
}

impl Children {
    pub(crate) fn of(node: &SyntaxNode) -> Self {
        let items: Vec<SyntaxElement> = node
            .children_with_tokens()
            .filter(|e| !is_trivia(elem_kind(e)))
            .collect();
        Self {
            items: items.into_iter(),
        }
    }

    pub(crate) fn peek(&self) -> Option<SyntaxElement> {
        self.items.as_slice().first().cloned()
    }

    pub(crate) fn peek_kind(&self) -> Option<SyntaxKind> {
        self.peek().as_ref().map(elem_kind)
    }

    pub(crate) fn next_elem(&mut self) -> Option<SyntaxElement> {
        self.items.next()
    }

    /// 次の要素がトークンで、かつ kind が一致すれば消費して返す。
    pub(crate) fn eat_token(&mut self, kind: SyntaxKind) -> Option<SyntaxToken> {
        if self.peek_kind() == Some(kind) {
            match self.next_elem() {
                Some(rowan::NodeOrToken::Token(t)) => Some(t),
                _ => None,
            }
        } else {
            None
        }
    }

    /// 次の要素がノードで、かつ kind が一致すれば消費して返す。
    pub(crate) fn eat_node(&mut self, kind: SyntaxKind) -> Option<SyntaxNode> {
        if self.peek_kind() == Some(kind) {
            match self.next_elem() {
                Some(rowan::NodeOrToken::Node(n)) => Some(n),
                _ => None,
            }
        } else {
            None
        }
    }

    /// 残りの要素の kind を並べる (trivia は既に除いてある)。
    pub(crate) fn into_kinds(self) -> Vec<SyntaxKind> {
        self.items.map(|e| elem_kind(&e)).collect()
    }

    /// 次の要素が何であれノードなら消費して返す (kind は問わない)。
    /// 式・型のように複数の kind を取りうる位置で使う。
    pub(crate) fn next_node(&mut self) -> Option<SyntaxNode> {
        match self.peek() {
            Some(rowan::NodeOrToken::Node(_)) => match self.next_elem() {
                Some(rowan::NodeOrToken::Node(n)) => Some(n),
                _ => unreachable!(),
            },
            _ => None,
        }
    }
}
