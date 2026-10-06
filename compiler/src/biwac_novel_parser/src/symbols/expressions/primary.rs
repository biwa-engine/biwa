use std::cell::OnceCell;

use biwac_span::Span;

use biwac_ast::{
    BoolLiteral, Exprs, FloatLiteral, FnLiteral, FnLiteralArg, Ident, IntegerLiteral, Literal,
    Primary, StringLiteral, StructLiteral, Variable,
};

use crate::{
    NCodeTokenOption, NovelParseError, NovelSourceStream,
    token::{NCodeTkKind, NCodeTkKindName},
};

// Primary = Literal | Identifier ( "(" ")" )? | "(" Exprs ")"
impl<'src> NovelSourceStream<'src> {
    pub(super) fn consume_primary_expression(&mut self) -> Result<Exprs, NovelParseError> {
        // Primary = Literal | "(" Expr ")"
        let t = self.peek_token()?.cloned().ok_or_else(|begin_idx| {
            NovelParseError::InvalidLineEnd {
                expecteds: vec![NCodeTkKindName::Ident, NCodeTkKindName::LiteralInteger],
                span: self.span_from(begin_idx, 1),
            }
        })?;

        match &t.kind {
            NCodeTkKind::LiteralInteger(int) => {
                self.next_token()?;
                Ok(Exprs::Primary(Primary::Literal(Literal::Integer(
                    IntegerLiteral {
                        val: *int,
                        span: t.span,
                    },
                ))))
            }
            NCodeTkKind::LiteralFloat(val) => {
                self.next_token()?;
                Ok(Exprs::Primary(Primary::Literal(Literal::Float(
                    FloatLiteral {
                        val: *val,
                        span: t.span,
                    },
                ))))
            }
            NCodeTkKind::LiteralString(str) => {
                self.next_token()?;
                Ok(Exprs::Primary(Primary::Literal(Literal::String(
                    StringLiteral {
                        val: str.to_string(),
                        span: t.span,
                    },
                ))))
            }
            NCodeTkKind::KwTrue => {
                self.next_token()?;
                Ok(Exprs::Primary(Primary::Literal(Literal::Bool(
                    BoolLiteral {
                        val: true,
                        span: t.span,
                    },
                ))))
            }
            NCodeTkKind::KwFalse => {
                self.next_token()?;
                Ok(Exprs::Primary(Primary::Literal(Literal::Bool(
                    BoolLiteral {
                        val: false,
                        span: t.span,
                    },
                ))))
            }
            // `package::` から始まる絶対パスも式に書ける。
            // `package` は識別子ではなくキーワードなので、ここで拾わないと
            // `consume_qualified_identifier` に辿り着けない。
            NCodeTkKind::Ident(_) | NCodeTkKind::KwPackage => {
                let begin = t.span.clone();
                let path = self.consume_qualified_identifier()?;

                // 直後の `(` は後置演算子 (呼び出し) として読む。
                if let NCodeTokenOption::Some(t2) = self.peek_token()? {
                    if let NCodeTkKind::MarkLBrace = t2.kind
                        && self.struct_literal_allowed()
                    {
                        let (members, span) = self.consume_struct_members()?;

                        Ok(Exprs::Primary(Primary::Literal(Literal::Struct(
                            StructLiteral {
                                path,
                                members,
                                span: Span::merge(&begin, &span),
                            },
                        ))))
                    } else {
                        Ok(Exprs::Primary(Primary::Variable(Variable::Path(path))))
                    }
                } else {
                    Ok(Exprs::Primary(Primary::Variable(Variable::Path(path))))
                }
            }
            NCodeTkKind::MarkLPare => {
                self.next_token()?;
                let expr = self.consume_delimited_expression()?;

                let _ = self.must_consume_next(vec![NCodeTkKindName::MarkRPare])?;

                Ok(expr)
            }
            NCodeTkKind::KwFn => Ok(Exprs::Primary(Primary::FnLiteral(
                self.consume_fn_literal()?,
            ))),
            _ => Err(NovelParseError::InvalidLineEnd {
                expecteds: vec![NCodeTkKindName::Ident, NCodeTkKindName::LiteralInteger],
                span: t.span,
            }),
        }
    }

    /// 無名関数 `fn ( <引数> ,* ) ( -> <型> )? { <式> }`。
    ///
    /// scene のコードは行単位で、本体に文を並べる形は書けない。
    /// 本体は式 1 つだけである (通常のコードでは文も書ける)。
    fn consume_fn_literal(&mut self) -> Result<FnLiteral, NovelParseError> {
        let begin = self
            .must_consume_next(vec![NCodeTkKindName::KwFn])?
            .span
            .clone();
        let _ = self.must_consume_next(vec![NCodeTkKindName::MarkLPare])?;

        let mut args = Vec::new();
        loop {
            if let NCodeTokenOption::Some(t) = self.peek_token()?
                && let NCodeTkKind::MarkRPare = t.kind
            {
                self.next_token()?;
                break;
            }
            let id = self.consume_identifier()?;
            let typ = if let NCodeTokenOption::Some(t) = self.peek_token()?
                && let NCodeTkKind::MarkColon = t.kind
            {
                self.next_token()?;
                Some(self.consume_type_representaion()?)
            } else {
                None
            };
            args.push(FnLiteralArg {
                id,
                typ,
                var_id: OnceCell::new(),
            });
            let t = self
                .must_consume_next(vec![NCodeTkKindName::MarkComma, NCodeTkKindName::MarkRPare])?;
            if let NCodeTkKind::MarkRPare = t.kind {
                break;
            }
        }

        let rtype = if let NCodeTokenOption::Some(t) = self.peek_token()?
            && let NCodeTkKind::MarkArrow = t.kind
        {
            self.next_token()?;
            Some(self.consume_type_representaion()?)
        } else {
            None
        };

        let _ = self.must_consume_next(vec![NCodeTkKindName::MarkLBrace])?;
        let expr = self.consume_delimited_expression()?;
        let end = self
            .must_consume_next(vec![NCodeTkKindName::MarkRBrace])?
            .span
            .clone();

        Ok(FnLiteral {
            args,
            rtype,
            stmts: Vec::new(),
            expr: Some(Box::new(expr)),
            span: Span::merge(&begin, &end),
        })
    }

    pub(super) fn consume_arguments(&mut self) -> Result<(Vec<Exprs>, Span), NovelParseError> {
        let begin = self
            .must_consume_next(vec![NCodeTkKindName::MarkLPare])?
            .span
            .clone();
        let mut span = begin.clone();

        let mut args: Vec<Exprs> = vec![];

        while let NCodeTokenOption::Some(t3) = self.peek_token()? {
            if let NCodeTkKind::MarkRPare = t3.kind {
                let end = t3.span.clone();
                span = Span::merge(&begin, &end);

                self.next_token()?;
                break;
            } else {
                let expr = self.consume_delimited_expression()?;
                args.push(expr);

                match self.peek_token()? {
                    NCodeTokenOption::Some(t) => {
                        if let NCodeTkKind::MarkComma = t.kind {
                            self.next_token()?;
                            continue;
                        } else if let NCodeTkKind::MarkRPare = t.kind {
                            continue;
                        } else {
                            return Err(NovelParseError::InvalidToken {
                                expecteds: vec![
                                    NCodeTkKindName::MarkRPare,
                                    NCodeTkKindName::MarkComma,
                                ],
                                found: Box::new(t.to_owned().clone()),
                            });
                        }
                    }
                    NCodeTokenOption::None { idx } => {
                        return Err(NovelParseError::InvalidLineEnd {
                            expecteds: vec![NCodeTkKindName::MarkRPare, NCodeTkKindName::MarkComma],
                            span: self.span_from(idx, 1),
                        });
                    }
                }
            }
        }

        Ok((args, span))
    }

    fn consume_struct_members(
        &mut self,
    ) -> Result<(Vec<(Ident, Box<Exprs>)>, Span), NovelParseError> {
        let begin = self
            .must_consume_next(vec![NCodeTkKindName::MarkLBrace])?
            .span
            .clone();
        let mut span = begin.clone();

        let mut members: Vec<(Ident, Box<Exprs>)> = vec![];

        while let NCodeTokenOption::Some(t3) = self.peek_token()? {
            if let NCodeTkKind::MarkRBrace = t3.kind {
                let end = t3.span.clone();
                span = Span::merge(&begin, &end);
                self.next_token()?;
                break;
            } else {
                let member = self.consume_identifier()?;
                let _ = self.must_consume_next(vec![NCodeTkKindName::MarkAssign])?;
                let expr = self.consume_delimited_expression()?;

                members.push((member, Box::new(expr)));

                match self.peek_token()? {
                    NCodeTokenOption::Some(t) => {
                        if let NCodeTkKind::MarkComma = t.kind {
                            self.next_token()?;
                            continue;
                        } else if let NCodeTkKind::MarkRBrace = t.kind {
                            continue;
                        } else {
                            return Err(NovelParseError::InvalidToken {
                                expecteds: vec![
                                    NCodeTkKindName::MarkRBrace,
                                    NCodeTkKindName::MarkComma,
                                ],
                                found: Box::new(t.to_owned().clone()),
                            });
                        }
                    }
                    NCodeTokenOption::None { idx } => {
                        return Err(NovelParseError::InvalidLineEnd {
                            expecteds: vec![
                                NCodeTkKindName::MarkRBrace,
                                NCodeTkKindName::MarkComma,
                            ],
                            span: self.span_from(idx, 1),
                        });
                    }
                }
            }
        }

        Ok((members, span))
    }
}
