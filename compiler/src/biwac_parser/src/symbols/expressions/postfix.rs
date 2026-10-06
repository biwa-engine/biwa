use biwac_lexer::TkKind;
use biwac_span::Span;

use biwac_ast::{CallExpr, Exprs, MemberAccess, Primary};

use crate::{ParseError, TokenStream};

impl<'t, 'src, 'i> TokenStream<'t, 'src, 'i> {
    pub(super) fn consume_postfix_expression(&mut self) -> Result<Exprs, ParseError<'src>> {
        let expr = self.consume_primary_expression()?;

        self.consume_postfix_after_expression(expr)
    }

    /// 後置演算子 (`.` <identifier> と `(` <引数列> `)`) を左結合で読む。
    ///
    /// 呼び出しの形は `<式> ( <引数列> )` の 1 つだけで、呼び先が何かは構文では決めない。
    /// `x.bar(a)` も「メンバアクセス `x.bar` の呼び出し」として読み、
    /// メソッドかメンバ (関数型) の値かは型推論が決める
    /// (`docs/function-as-the-first-class-type-impl-status.md` §7)。
    fn consume_postfix_after_expression(&mut self, expr: Exprs) -> Result<Exprs, ParseError<'src>> {
        let Some(t) = self.peek() else {
            return Ok(expr);
        };

        match t.kind {
            TkKind::MarkDot => {
                self.next();

                let member = self.consume_identifier()?;

                self.consume_postfix_after_expression(Exprs::Primary(Primary::MemberAccess(
                    MemberAccess {
                        left: Box::new(expr),
                        member,
                    },
                )))
            }
            TkKind::MarkLPare => {
                let (args, span) = self.consume_arguments()?;

                self.consume_postfix_after_expression(Exprs::Primary(Primary::Call(CallExpr {
                    span: Span::merge(&expr.span(), &span),
                    callee: Box::new(expr),
                    args,
                })))
            }
            _ => Ok(expr),
        }
    }
}
