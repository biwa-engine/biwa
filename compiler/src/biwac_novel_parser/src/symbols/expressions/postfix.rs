use biwac_span::Span;

use biwac_ast::{CallExpr, Exprs, MemberAccess, MethodCall, Primary};

use crate::{NCodeTokenOption, NovelParseError, NovelSourceStream, token::NCodeTkKind};

impl<'src> NovelSourceStream<'src> {
    pub(super) fn consume_postfix_expression(&mut self) -> Result<Exprs, NovelParseError> {
        let expr = self.consume_primary_expression()?;

        self.consume_postfix_after_expression(expr)
    }

    fn consume_postfix_after_expression(&mut self, expr: Exprs) -> Result<Exprs, NovelParseError> {
        if let NCodeTokenOption::Some(t) = self.peek_token()? {
            match t.kind {
                NCodeTkKind::MarkDot => {
                    self.next_token()?;

                    let mem_or_method = self.consume_identifier()?;

                    if let NCodeTokenOption::Some(t) = self.peek_token()? {
                        if let NCodeTkKind::MarkLPare = t.kind {
                            let (args, span) = self.consume_arguments()?;

                            // メソッド呼び出しの後ろにも後置演算子が続きうる。
                            // `a.b().c()` や `a.b().c` を切らないよう、ここでも再帰する。
                            Ok(self.consume_postfix_after_expression(Exprs::Primary(
                                Primary::MethodCall(MethodCall {
                                    span: Span::merge(&expr.span(), &span),
                                    left: Box::new(expr),
                                    method: mem_or_method,
                                    args,
                                }),
                            ))?)
                        } else {
                            Ok(self.consume_postfix_after_expression(Exprs::Primary(
                                Primary::MemberAccess(MemberAccess {
                                    left: Box::new(expr),
                                    member: mem_or_method.clone(),
                                }),
                            ))?)
                        }
                    } else {
                        Ok(self.consume_postfix_after_expression(Exprs::Primary(
                            Primary::MemberAccess(MemberAccess {
                                left: Box::new(expr),
                                member: mem_or_method.clone(),
                            }),
                        ))?)
                    }
                }
                // 任意の式の後ろの `(` は呼び出し (`make()(x)`、`(f)(x)`)。
                // 通常のパーサーと同じ規則である。
                //
                // 埋め込み式 (`$f(x)(テキスト)`) の範囲は字句の段 (`scan.rs`) が
                // 最初の呼び出しの `)` までで切るので、ここには届かない。
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
        } else {
            Ok(expr)
        }
    }
}
