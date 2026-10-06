use biwac_ast::{DefTyp, PrimTyp, TypRepr, TypReprVal};

use crate::{
    NCodeTokenOption, NovelParseError, NovelSourceStream,
    token::{NCodeTkKind, NCodeTkKindName},
};

impl<'src> NovelSourceStream<'src> {
    pub(crate) fn opt_consume_type_annotation(
        &mut self,
    ) -> Result<Option<TypRepr>, NovelParseError> {
        let t = self
            .peek_token()?
            .cloned()
            .ok_or_else(|begin_idx| NovelParseError::InvalidLineEnd {
                expecteds: vec![NCodeTkKindName::MarkColon],
                span: self.span_from(begin_idx, 1),
            })?
            .to_owned();

        if let NCodeTkKind::MarkColon = t.kind {
            self.next_token()?;

            Ok(Some(self.consume_type_representaion()?))
        } else {
            Ok(None)
        }
    }

    pub(crate) fn consume_type_representaion(&mut self) -> Result<TypRepr, NovelParseError> {
        match self.peek_token()? {
            NCodeTokenOption::Some(t) => {
                if let NCodeTkKind::KwUint = t.kind {
                    let span = t.span.clone();
                    self.next_token()?;
                    Ok(TypRepr {
                        val: TypReprVal::Primitive(PrimTyp::Uint),
                        span,
                    })
                } else if let NCodeTkKind::KwInt = t.kind {
                    let span = t.span.clone();
                    self.next_token()?;
                    Ok(TypRepr {
                        val: TypReprVal::Primitive(PrimTyp::Int),
                        span,
                    })
                } else if let NCodeTkKind::KwFloat = t.kind {
                    let span = t.span.clone();
                    self.next_token()?;
                    Ok(TypRepr {
                        val: TypReprVal::Primitive(PrimTyp::Float),
                        span,
                    })
                } else if let NCodeTkKind::KwBool = t.kind {
                    let span = t.span.clone();
                    self.next_token()?;
                    Ok(TypRepr {
                        val: TypReprVal::Primitive(PrimTyp::Bool),
                        span,
                    })
                } else if let NCodeTkKind::Ident(_) = t.kind {
                    // NOTE: idのみ得られた場合、ジェネリクス型(`T`)である可能性がある
                    let path = self.consume_qualified_identifier()?;
                    let genargs = self.opt_consume_generic_args()?;

                    Ok(TypRepr {
                        span: path.span(),
                        val: TypReprVal::Defined(DefTyp { path, genargs }),
                    })
                } else if let NCodeTkKind::KwPackage = t.kind {
                    let path = self.consume_qualified_identifier()?;
                    let genargs = self.opt_consume_generic_args()?;

                    Ok(TypRepr {
                        span: path.span(),
                        val: TypReprVal::Defined(DefTyp { path, genargs }),
                    })
                } else if let NCodeTkKind::KwFn = t.kind {
                    self.consume_fn_type_representation()
                } else {
                    Err(NovelParseError::InvalidToken {
                        expecteds: vec![
                            NCodeTkKindName::KwUint,
                            NCodeTkKindName::KwInt,
                            NCodeTkKindName::KwBool,
                            NCodeTkKindName::Ident,
                            NCodeTkKindName::KwFn,
                        ],

                        found: Box::new(t.to_owned().clone()),
                    })
                }
            }
            NCodeTokenOption::None { idx } => Err(NovelParseError::InvalidLineEnd {
                expecteds: vec![
                    NCodeTkKindName::KwUint,
                    NCodeTkKindName::KwInt,
                    NCodeTkKindName::KwBool,
                    NCodeTkKindName::Ident,
                    NCodeTkKindName::KwPackage,
                ],
                span: self.span_from(idx, 1),
            }),
        }
    }

    /// 関数型 `fn(A, B) -> C` / `fn(A)` を読む (通常のパーサーの同名の関数と同じ規則)。
    /// 型の中に量化子は持てないので、`fn` の直後には `(` を要求する。
    fn consume_fn_type_representation(&mut self) -> Result<TypRepr, NovelParseError> {
        let begin = self
            .must_consume_next(vec![NCodeTkKindName::KwFn])?
            .span
            .clone();
        let _ = self.must_consume_next(vec![NCodeTkKindName::MarkLPare])?;

        let mut args = Vec::new();
        let mut end = loop {
            if let NCodeTokenOption::Some(t) = self.peek_token()?
                && let NCodeTkKind::MarkRPare = t.kind
            {
                break self
                    .must_consume_next(vec![NCodeTkKindName::MarkRPare])?
                    .span
                    .clone();
            }
            args.push(self.consume_type_representaion()?);
            let t = self
                .must_consume_next(vec![NCodeTkKindName::MarkComma, NCodeTkKindName::MarkRPare])?;
            if let NCodeTkKind::MarkRPare = t.kind {
                break t.span.clone();
            }
        };

        let rty = if let NCodeTokenOption::Some(t) = self.peek_token()?
            && let NCodeTkKind::MarkArrow = t.kind
        {
            self.must_consume_next(vec![NCodeTkKindName::MarkArrow])?;
            let rty = self.consume_type_representaion()?;
            end = rty.span.clone();
            Some(Box::new(rty))
        } else {
            None
        };

        Ok(TypRepr {
            val: TypReprVal::Fn(biwac_ast::FnTyp { args, rty }),
            span: biwac_span::Span::merge(&begin, &end),
        })
    }

    /// Optionaly consumes tokens and parses to get generic arguments.
    /// We should use here:
    /// let a: foo::bar[Int] = ...
    ///                ^
    ///                |
    // pub(crate) fn opt_consume_generic_argument_assignment(
    pub(crate) fn opt_consume_generic_args(
        &mut self,
    ) -> Result<Option<Vec<TypRepr>>, NovelParseError> {
        if let NCodeTokenOption::Some(t) = self.peek_token()?
            && matches!(t.kind, NCodeTkKind::MarkLBracket)
        {
            self.next_token()?;
        } else {
            return Ok(None);
        }

        let mut genargs = vec![];
        loop {
            if let NCodeTokenOption::Some(t) = self.peek_token()?
                && let NCodeTkKind::MarkRBracket = t.kind
            {
                self.next_token()?;

                return Ok(Some(genargs));
            } else {
                genargs.push(self.consume_type_representaion()?);

                match self.next_token()? {
                    NCodeTokenOption::Some(t) => {
                        if let NCodeTkKind::MarkRBracket = t.kind {
                            return Ok(Some(genargs));
                        } else if let NCodeTkKind::MarkComma = t.kind {
                            continue;
                        } else {
                            return Err(NovelParseError::InvalidToken {
                                expecteds: vec![
                                    NCodeTkKindName::MarkRBracket,
                                    NCodeTkKindName::MarkComma,
                                ],
                                found: Box::new(t.clone()),
                            });
                        }
                    }
                    NCodeTokenOption::None { idx } => {
                        return Err(NovelParseError::InvalidLineEnd {
                            expecteds: vec![
                                NCodeTkKindName::MarkRBracket,
                                NCodeTkKindName::MarkComma,
                            ],
                            span: self.span_from(idx, 1),
                        });
                    }
                }
            }
        }
    }
}
