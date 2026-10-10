use biwac_lexer::{TkKind, TkKindName};
use biwac_span::Span;

use biwac_ast::{
    AbsolutePathHeader, BoolLiteral, Exprs, FloatLiteral, FnLiteral, FnLiteralArg, Ident,
    IntegerLiteral, Literal, Path, Primary, SelfTypHeader, StringLiteral, StructLiteral, Variable,
};

use std::cell::OnceCell;

use crate::{ParseError, TokenStream, symbols::statements::ExprOrStmt};

// Primary = Literal | Identifier ( "(" ")" )? | "(" Exprs ")"
impl<'t, 'src, 'i> TokenStream<'t, 'src, 'i> {
    pub(super) fn consume_primary_expression(&mut self) -> Result<Exprs, ParseError<'src>> {
        let mod_id = self.mod_id;

        // Primary = Literal | "(" Expr ")"
        let t = *self.peek().ok_or(ParseError::InvalidEOF {
            mod_id,
            expecteds: vec![TkKindName::Ident, TkKindName::LiteralInteger],
        })?;

        match &t.kind {
            TkKind::LiteralInteger(val) => {
                self.next();
                Ok(Exprs::Primary(Primary::Literal(Literal::Integer(
                    IntegerLiteral {
                        val: *val,
                        span: t.span.clone(),
                    },
                ))))
            }
            TkKind::LiteralFloat(val) => {
                self.next();
                Ok(Exprs::Primary(Primary::Literal(Literal::Float(
                    FloatLiteral {
                        val: *val,
                        span: t.span.clone(),
                    },
                ))))
            }
            TkKind::LiteralString(str) => {
                self.next();
                Ok(Exprs::Primary(Primary::Literal(Literal::String(
                    StringLiteral {
                        val: str.to_string(),
                        span: t.span.clone(),
                    },
                ))))
            }
            TkKind::KwBoolTrue => {
                self.next();
                Ok(Exprs::Primary(Primary::Literal(Literal::Bool(
                    BoolLiteral {
                        val: true,
                        span: t.span.clone(),
                    },
                ))))
            }
            TkKind::KwBoolFalse => {
                self.next();
                Ok(Exprs::Primary(Primary::Literal(Literal::Bool(
                    BoolLiteral {
                        val: false,
                        span: t.span.clone(),
                    },
                ))))
            }
            // `package::` から始まる絶対パスも式に書ける。
            // `package` は識別子ではなくキーワードなので、ここで拾わないと
            // `consume_qualified_identifier` に辿り着けない。
            TkKind::Ident(_) | TkKind::KwPackage | TkKind::KwSuper => {
                let begin = t.span.clone();
                let path = self.consume_qualified_identifier()?;

                // 直後の `(` は後置演算子 (呼び出し) として読む。
                if let Some(t2) = self.peek() {
                    if let TkKind::MarkLBrace = t2.kind
                        && !self.no_struct_literal
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
            TkKind::KwSelfTyp => {
                let begin = t.span.clone();

                // "Self" (
                //   ( "::" <identifier> ) |
                //   ( "{" ... "}" )
                // )
                //
                // `Self::new` はパスの値で、`(..)` が続けば後置演算子の呼び出しになる。
                self.next();

                if let Some(t) = self.peek().copied() {
                    match t.kind {
                        TkKind::MarkDoubleColon => {
                            self.next();

                            let ident = self.consume_identifier()?;

                            Ok(Exprs::Primary(Primary::Variable(Variable::Path(
                                Path::new(
                                    Some(AbsolutePathHeader::SelfTyp(SelfTypHeader::new(
                                        begin.clone(),
                                    ))),
                                    vec![ident.into()],
                                ),
                            ))))
                        }
                        TkKind::MarkLBrace if !self.no_struct_literal => {
                            let (members, span) = self.consume_struct_members()?;

                            Ok(Exprs::Primary(Primary::Literal(Literal::Struct(
                                StructLiteral {
                                    members,
                                    span: Span::merge(&begin, &span),
                                    path: Path::new(
                                        Some(AbsolutePathHeader::SelfTyp(SelfTypHeader::new(
                                            begin.clone(),
                                        ))),
                                        Vec::new(),
                                    ),
                                },
                            ))))
                        }
                        _ => Err(ParseError::InvalidToken {
                            expecteds: vec![TkKindName::MarkDoubleColon, TkKindName::MarkLBrace],
                            found: t.clone(),
                        }),
                    }
                } else {
                    Err(ParseError::InvalidEOF {
                        mod_id,
                        expecteds: vec![TkKindName::MarkDoubleColon, TkKindName::MarkLBrace],
                    })
                }
            }
            TkKind::KwSelfVar => {
                let self_span = t.span.clone();

                // "self"
                self.next();
                Ok(Exprs::Primary(Primary::Variable(Variable::SelfVar(
                    self_span,
                ))))
            }
            // `let n = match o { .. };` のように、値が要る場所に書ける。
            // ここでは必ず式形として読む。
            TkKind::KwMatch => Ok(Exprs::Primary(Primary::Match(
                self.consume_match_expression()?,
            ))),
            // 無名関数。型の位置の `fn(A) -> B` とは、式の位置に現れることで区別される。
            TkKind::KwFn => Ok(Exprs::Primary(Primary::FnLiteral(
                self.consume_fn_literal()?,
            ))),
            TkKind::MarkLPare => {
                self.next();
                let expr = self.consume_delimited_expression()?;

                let _ = self.must_consume_next(vec![TkKindName::MarkRPare])?;

                Ok(expr)
            }
            // 実際にはトークンがある。EOF として報告すると
            // 位置がファイル末尾になって原因が追えないので、そのトークンを指す。
            _ => Err(ParseError::InvalidToken {
                expecteds: vec![
                    TkKindName::Ident,
                    TkKindName::LiteralInteger,
                    TkKindName::LiteralFloat,
                    TkKindName::LiteralString,
                    TkKindName::KwBoolTrue,
                    TkKindName::KwBoolFalse,
                    TkKindName::MarkLPare,
                ],
                found: t.clone(),
            }),
        }
    }

    /// 無名関数 `fn ( <引数> ,* ) ( -> <型> )? <ブロック>`。引数の型は省略できる。
    fn consume_fn_literal(&mut self) -> Result<FnLiteral, ParseError<'src>> {
        let begin = self.must_consume_next(vec![TkKindName::KwFn])?.span.clone();
        let _ = self.must_consume_next(vec![TkKindName::MarkLPare])?;

        let mut args = Vec::new();
        loop {
            if let Some(t) = self.peek()
                && let TkKind::MarkRPare = t.kind
            {
                self.next();
                break;
            }
            let id = self.consume_identifier()?;
            let typ = if self
                .consume_next_if_match(vec![TkKindName::MarkColon])
                .is_some()
            {
                Some(self.consume_type_representaion()?)
            } else {
                None
            };
            args.push(FnLiteralArg {
                id,
                typ,
                var_id: OnceCell::new(),
            });
            let t = self.must_consume_next(vec![TkKindName::MarkComma, TkKindName::MarkRPare])?;
            if let TkKind::MarkRPare = t.kind {
                break;
            }
        }

        let rtype = if self
            .consume_next_if_match(vec![TkKindName::MarkArrow])
            .is_some()
        {
            Some(self.consume_type_representaion()?)
        } else {
            None
        };

        // 本体の中では構造体リテラルを書いてよい (条件式の中に書かれていても)。
        let saved = self.no_struct_literal;
        self.no_struct_literal = false;
        let body = self.consume_block_expression_or_statement();
        self.no_struct_literal = saved;
        let (stmts, expr, end) = match body? {
            ExprOrStmt::Expr(block_expr) => {
                (block_expr.stmts, Some(block_expr.expr), block_expr.span)
            }
            ExprOrStmt::Stmt(block_stmt) => (block_stmt.stmts, None, block_stmt.span),
        };

        Ok(FnLiteral {
            args,
            rtype,
            stmts,
            expr,
            span: Span::merge(&begin, &end),
        })
    }

    pub(super) fn consume_arguments(&mut self) -> Result<(Vec<Exprs>, Span), ParseError<'src>> {
        let begin = self
            .must_consume_next(vec![TkKindName::MarkLPare])?
            .span
            .clone();
        let mut span = begin.clone();

        let mut args: Vec<Exprs> = vec![];

        while let Some(t3) = self.peek() {
            if let TkKind::MarkRPare = t3.kind {
                let end = t3.span.clone();
                span = Span::merge(&begin, &end);

                self.next();
                break;
            } else {
                let expr = self.consume_delimited_expression()?;
                args.push(expr);

                if let Some(t) = self.peek() {
                    if let TkKind::MarkComma = t.kind {
                        self.next();
                        continue;
                    } else if let TkKind::MarkRPare = t.kind {
                        continue;
                    } else {
                        return Err(ParseError::InvalidToken {
                            expecteds: vec![TkKindName::MarkRPare, TkKindName::MarkComma],
                            found: t.to_owned().clone(),
                        });
                    }
                } else {
                    return Err(ParseError::InvalidEOF {
                        mod_id: self.mod_id,
                        expecteds: vec![TkKindName::MarkRPare, TkKindName::MarkComma],
                    });
                }
            }
        }

        Ok((args, span))
    }

    fn consume_struct_members(
        &mut self,
    ) -> Result<(Vec<(Ident, Box<Exprs>)>, Span), ParseError<'src>> {
        let begin = self
            .must_consume_next(vec![TkKindName::MarkLBrace])?
            .span
            .clone();
        let mut span = begin.clone();

        let mut members: Vec<(Ident, Box<Exprs>)> = vec![];

        while let Some(t3) = self.peek() {
            if let TkKind::MarkRBrace = t3.kind {
                let end = t3.span.clone();
                span = Span::merge(&begin, &end);
                self.next();
                break;
            } else {
                let member = self.consume_identifier()?;
                let _ = self.must_consume_next(vec![TkKindName::MarkAssign])?;
                let expr = self.consume_delimited_expression()?;

                members.push((member, Box::new(expr)));

                if let Some(t) = self.peek() {
                    if let TkKind::MarkComma = t.kind {
                        self.next();
                        continue;
                    } else if let TkKind::MarkRBrace = t.kind {
                        continue;
                    } else {
                        return Err(ParseError::InvalidToken {
                            expecteds: vec![TkKindName::MarkRBrace, TkKindName::MarkComma],
                            found: t.to_owned().clone(),
                        });
                    }
                } else {
                    return Err(ParseError::InvalidEOF {
                        mod_id: self.mod_id,
                        expecteds: vec![TkKindName::MarkRBrace, TkKindName::MarkComma],
                    });
                }
            }
        }

        Ok((members, span))
    }
}
