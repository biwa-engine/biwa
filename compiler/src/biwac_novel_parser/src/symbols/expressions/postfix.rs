use biwac_span::Span;

use biwac_ast::{CallExpr, Exprs, MemberAccess, Primary};

use crate::{NCodeTokenOption, NovelParseError, NovelSourceStream, token::NCodeTkKind};

impl<'src> NovelSourceStream<'src> {
    pub(super) fn consume_postfix_expression(&mut self) -> Result<Exprs, NovelParseError> {
        let expr = self.consume_primary_expression()?;

        self.consume_postfix_after_expression(expr)
    }

    /// 後置演算子 (`.` <identifier> と `(` <引数列> `)`) を左結合で読む。
    /// 通常のパーサーと同じ規則である (呼び出しの形は `<式> ( <引数列> )` の 1 つだけ)。
    ///
    /// 埋め込み式 (`$f(x)(テキスト)`) の範囲は字句の段 (`scan.rs`) が
    /// 最初の呼び出しの `)` までで切るので、後ろの `(..)` はここには届かない。
    fn consume_postfix_after_expression(&mut self, expr: Exprs) -> Result<Exprs, NovelParseError> {
        let NCodeTokenOption::Some(t) = self.peek_token()? else {
            return Ok(expr);
        };

        match t.kind {
            NCodeTkKind::MarkDot => {
                self.next_token()?;

                let member = self.consume_identifier()?;

                self.consume_postfix_after_expression(Exprs::Primary(Primary::MemberAccess(
                    MemberAccess {
                        left: Box::new(expr),
                        member,
                    },
                )))
            }
            NCodeTkKind::MarkLPare => {
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
