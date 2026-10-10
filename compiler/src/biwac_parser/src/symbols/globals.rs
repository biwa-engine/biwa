use std::cell::OnceCell;

use biwac_lexer::{TkKindName, token::TkKind};
use biwac_span::Span;

use biwac_ast::{
    ArgDecl, ArgDeclList, Attrs, EnumDef, FnDef, Globals, Ident, ImplBlock, ImportDecl,
    MethodArgDeclList, MethodDef, ModDecl, NativeCode, NativeFnDef, NativeMethodDef,
    NativeTypeAlias, NovelScene, RetTypRepr, StructDef, StructMemberDecl, TraitDef, TraitItemArgs,
    TraitItemDecl, TypRepr, TypeAlias, TypeDef, VariantDecl, VariantFieldsDecl, Visibility,
};

use crate::{ExprOrStmt, ParseError, TokenStream};

enum FnOrMethod<F, M> {
    Fn(F),
    Method(M),
}

enum CodeOrNative<C, N> {
    Code(C),
    Native(N),
}

impl<'t, 'src, 'i> TokenStream<'t, 'src, 'i> {
    fn consume_return_type(
        &mut self,
        arg_decl_span: &Span,
    ) -> Result<RetTypRepr, ParseError<'src>> {
        if self
            .consume_next_if_match(vec![TkKindName::MarkArrow])
            .is_some()
        {
            Ok(RetTypRepr::Typ(self.consume_type_representaion()?))
        } else {
            Ok(RetTypRepr::Void(Span::new(
                arg_decl_span.module(),
                arg_decl_span.end(),
                arg_decl_span.end(),
            )))
        }
    }

    fn consume_function(
        &mut self,
        attrs: Attrs,
        vis: Visibility,
    ) -> Result<CodeOrNative<FnDef, NativeFnDef>, ParseError<'src>> {
        let begin = self.must_consume_next(vec![TkKindName::KwFn])?.span.clone();

        let id = self.consume_identifier()?;

        let genargs = self.opt_consume_generic_argument_declaration()?;

        let args = self.consume_argsdec()?;
        let rtype = self.consume_return_type(&args.span)?;
        if self.has_native_attr(&attrs) {
            if let Some(t) = self.next() {
                if let TkKind::DslLiteral(str) = t.kind {
                    Ok(CodeOrNative::Native(NativeFnDef {
                        vis: vis.clone(),
                        id,
                        def_id: OnceCell::new(),
                        args,
                        native: str.to_string(),
                        rtype,
                        span: Span::merge(&begin, &t.span),
                        native_span: t.span.clone(),
                        attrs,
                        genargs,
                    }))
                } else {
                    Err(ParseError::InvalidToken {
                        expecteds: vec![TkKindName::DslLiteral],
                        found: t.clone(),
                    })
                }
            } else {
                Err(ParseError::InvalidEOF {
                    mod_id: self.mod_id,
                    expecteds: vec![TkKindName::DslLiteral],
                })
            }
        } else {
            let (stmts, expr, end) = match self.consume_block_expression_or_statement()? {
                ExprOrStmt::Expr(block_expr) => {
                    (block_expr.stmts, Some(*block_expr.expr), block_expr.span)
                }
                ExprOrStmt::Stmt(block_stmt) => (block_stmt.stmts, None, block_stmt.span),
            };

            Ok(CodeOrNative::Code(FnDef {
                vis: vis.clone(),
                id,
                def_id: OnceCell::new(),
                args,
                stmts,
                expr,
                rtype,
                span: Span::merge(&begin, &end),
                attrs,
                genargs,
            }))
        }
    }

    /// trait が宣言する項目。
    ///
    /// ```biwa
    /// fn gyao(self) -> Gyoe;
    /// fn guee(aaa: Aaa) -> Self;
    /// ```
    ///
    /// 本体は書かない。`self` を取るかどうかで
    /// メソッド形式と関連関数形式に分かれる。
    fn consume_trait_item(&mut self) -> Result<TraitItemDecl, ParseError<'src>> {
        let attrs = self.consume_attributes()?;
        // trait の項目は trait と同じ可視性になる (Rust と同じ)。
        let vis = self.opt_consume_visibility()?;
        reject_visibility(
            &vis,
            "an item of a trait (it has the same visibility as the trait)",
        )?;
        let begin = self.must_consume_next(vec![TkKindName::KwFn])?.span.clone();

        let id = self.consume_identifier()?;
        let genargs = self.opt_consume_generic_argument_declaration()?;

        let args = match self.consume_method_argsdec()? {
            FnOrMethod::Fn(args) => TraitItemArgs::Assoc(args),
            FnOrMethod::Method(args) => TraitItemArgs::Method(args),
        };
        let args_span = match &args {
            TraitItemArgs::Assoc(a) => a.span.clone(),
            TraitItemArgs::Method(a) => a.span.clone(),
        };

        let rtype = self.consume_return_type(&args_span)?;

        let end = self
            .must_consume_next(vec![TkKindName::MarkSemiColon])?
            .span
            .clone();

        Ok(TraitItemDecl {
            id,
            def_id: OnceCell::new(),
            args,
            rtype,
            genargs,
            attrs,
            span: Span::merge(&begin, &end),
        })
    }

    fn consume_function_or_method_definition(
        &mut self,
        attrs: Attrs,
        vis: Visibility,
    ) -> Result<
        FnOrMethod<CodeOrNative<FnDef, NativeFnDef>, CodeOrNative<MethodDef, NativeMethodDef>>,
        ParseError<'src>,
    > {
        let begin = self.must_consume_next(vec![TkKindName::KwFn])?.span.clone();

        let id = self.consume_identifier()?;

        let genargs = self.opt_consume_generic_argument_declaration()?;

        match self.consume_method_argsdec()? {
            FnOrMethod::Fn(args) => {
                let rtype = self.consume_return_type(&args.span)?;
                if self.has_native_attr(&attrs) {
                    if let Some(t) = self.next() {
                        if let TkKind::DslLiteral(str) = t.kind {
                            Ok(FnOrMethod::Fn(CodeOrNative::Native(NativeFnDef {
                                vis: vis.clone(),
                                id,
                                def_id: OnceCell::new(),
                                args,
                                native: str.to_string(),
                                rtype,
                                span: Span::merge(&begin, &t.span),
                                native_span: t.span.clone(),
                                attrs,
                                genargs,
                            })))
                        } else {
                            Err(ParseError::InvalidToken {
                                expecteds: vec![TkKindName::DslLiteral],
                                found: t.clone(),
                            })
                        }
                    } else {
                        Err(ParseError::InvalidEOF {
                            mod_id: self.mod_id,
                            expecteds: vec![TkKindName::DslLiteral],
                        })
                    }
                } else {
                    let (stmts, expr, end) = match self.consume_block_expression_or_statement()? {
                        ExprOrStmt::Expr(block_expr) => {
                            (block_expr.stmts, Some(*block_expr.expr), block_expr.span)
                        }
                        ExprOrStmt::Stmt(block_stmt) => (block_stmt.stmts, None, block_stmt.span),
                    };

                    Ok(FnOrMethod::Fn(CodeOrNative::Code(FnDef {
                        vis: vis.clone(),
                        id,
                        def_id: OnceCell::new(),
                        args,
                        stmts,
                        expr,
                        rtype,
                        span: Span::merge(&begin, &end),
                        attrs,
                        genargs,
                    })))
                }
            }
            FnOrMethod::Method(args) => {
                let rtype = self.consume_return_type(&args.span)?;
                if self.has_native_attr(&attrs) {
                    if let Some(t) = self.next() {
                        if let TkKind::DslLiteral(str) = t.kind {
                            Ok(FnOrMethod::Method(CodeOrNative::Native(NativeMethodDef {
                                vis: vis.clone(),
                                id,
                                def_id: OnceCell::new(),
                                args,
                                rtype,
                                native: str.to_string(),
                                native_span: t.span.clone(),
                                span: Span::merge(&begin, &t.span),
                                attrs,
                                genargs,
                            })))
                        } else {
                            Err(ParseError::InvalidToken {
                                expecteds: vec![TkKindName::DslLiteral],
                                found: t.clone(),
                            })
                        }
                    } else {
                        Err(ParseError::InvalidEOF {
                            mod_id: self.mod_id,
                            expecteds: vec![TkKindName::DslLiteral],
                        })
                    }
                } else {
                    let (stmts, expr, end) = match self.consume_block_expression_or_statement()? {
                        ExprOrStmt::Expr(block_expr) => {
                            (block_expr.stmts, Some(*block_expr.expr), block_expr.span)
                        }
                        ExprOrStmt::Stmt(block_stmt) => (block_stmt.stmts, None, block_stmt.span),
                    };
                    Ok(FnOrMethod::Method(CodeOrNative::Code(MethodDef {
                        vis: vis.clone(),
                        id,
                        def_id: OnceCell::new(),
                        args,
                        stmts,
                        expr,
                        rtype,
                        span: Span::merge(&begin, &end),
                        attrs,
                        genargs,
                    })))
                }
            }
        }
    }

    pub(super) fn opt_consume_global_symbols(
        &mut self,
    ) -> Result<Option<Globals>, ParseError<'src>> {
        let mod_id = self.mod_id;
        let attrs = self.consume_attributes()?;
        let vis = self.opt_consume_visibility()?;

        if let Some(t) = self.peek() {
            match t.kind {
                TkKind::KwImport => {
                    // "import" <qualified-identifier> ";"
                    // `pub import` (再 export) はまだ無い。
                    reject_visibility(&vis, "an import (`pub import` is not supported yet)")?;
                    let begin = t.span.clone();
                    self.next();
                    let path = self.consume_qualified_identifier()?;

                    // ";"
                    let end = self.must_consume_semicolon()?.span.clone();

                    Ok(Some(Globals::Import(ImportDecl {
                        path,
                        span: Span::merge(&begin, &end),
                    })))
                }
                TkKind::KwMod => {
                    // <visibility>? "mod" <identifier> ";"
                    if let Some(attr) = attrs.iter().next() {
                        return Err(ParseError::NotAllowedHere {
                            span: attr.span.clone(),
                            what: "an attribute",
                            place: "a module declaration",
                        });
                    }
                    let begin = vis.span().cloned().unwrap_or_else(|| t.span.clone());
                    self.next();
                    let id = self.consume_identifier()?;
                    let end = self.must_consume_semicolon()?.span.clone();

                    Ok(Some(Globals::Mod(ModDecl {
                        vis,
                        id,
                        span: Span::merge(&begin, &end),
                    })))
                }
                TkKind::KwFn => Ok(match self.consume_function(attrs, vis)? {
                    CodeOrNative::Code(f) => Some(Globals::FnDef(f)),
                    CodeOrNative::Native(f) => Some(Globals::NativeFnDef(f)),
                }),
                TkKind::KwStruct => {
                    // "struct" <identifier> <generic-argument-declaration>?
                    //     "{" ( <visibility>? <identifier> ":" <type> "," )* "}"
                    self.next();

                    let id = self.consume_identifier()?;

                    let genargs = self.opt_consume_generic_argument_declaration()?;

                    let _ = self.must_consume_next(vec![TkKindName::MarkLBrace])?;

                    let mut members = vec![];

                    loop {
                        let t = self.peek().ok_or(ParseError::InvalidEOF {
                            mod_id,
                            expecteds: vec![TkKindName::Ident, TkKindName::MarkRBrace],
                        })?;

                        if let TkKind::MarkRBrace = t.kind {
                            self.next();

                            return Ok(Some(Globals::TypeDef(TypeDef::Struct(StructDef {
                                vis,
                                id,
                                def_id: OnceCell::new(),
                                members,
                                genargs,
                                attrs,
                            }))));
                        } else {
                            let member_vis = self.opt_consume_visibility()?;
                            let t = self
                                .next()
                                .ok_or(ParseError::InvalidEOF {
                                    mod_id,
                                    expecteds: vec![TkKindName::Ident],
                                })?
                                .to_owned();
                            if let TkKind::Ident(member_id) = &t.kind {
                                let t = t.clone();
                                // WARN: really?
                                let typ = self.must_consume_type_annotation()?;

                                members.push(StructMemberDecl {
                                    vis: member_vis,
                                    id: Ident {
                                        id: *member_id,
                                        span: t.span,
                                    },
                                    typ,
                                });

                                let t = self.must_consume_next(vec![
                                    TkKindName::MarkComma,
                                    TkKindName::MarkRBrace,
                                ])?;
                                if let TkKind::MarkComma = t.kind {
                                    continue;
                                } else if let TkKind::MarkRBrace = t.kind {
                                    return Ok(Some(Globals::TypeDef(TypeDef::Struct(
                                        StructDef {
                                            vis,
                                            id,
                                            def_id: OnceCell::new(),
                                            members,
                                            genargs,
                                            attrs,
                                        },
                                    ))));
                                }
                            } else {
                                return Err(ParseError::InvalidToken {
                                    expecteds: vec![TkKindName::Ident],
                                    found: t.clone(),
                                });
                            }
                        }
                    }
                }
                TkKind::KwEnum => {
                    // "enum" <identifier> ( <generic-argument-declaration> )?
                    //     "{" ( <variant> "," )* "}"
                    self.next();

                    let id = self.consume_identifier()?;
                    let genargs = self.opt_consume_generic_argument_declaration()?;
                    let variants = self.consume_variant_declarations()?;

                    Ok(Some(Globals::TypeDef(TypeDef::Enum(EnumDef {
                        vis,
                        id,
                        def_id: OnceCell::new(),
                        variants,
                        genargs,
                        attrs,
                    }))))
                }
                TkKind::KwType => {
                    if self.has_native_attr(&attrs) {
                        // "type" <identifier> ( <generic-argument-declaration> )?
                        //     "=" {{
                        //         native type implementation
                        //     }} ";"

                        self.next();

                        let ident = self.consume_identifier()?;

                        let genargs = self.opt_consume_generic_argument_declaration()?;

                        let _ = self.must_consume_next(vec![TkKindName::MarkAssign])?;

                        let t = self.next().ok_or(ParseError::InvalidEOF {
                            mod_id,
                            expecteds: vec![TkKindName::DslLiteral],
                        })?;
                        if let TkKind::DslLiteral(str) = t.kind {
                            let native = str.to_string();
                            let native_span = t.span.clone();

                            let _ = self.must_consume_next(vec![TkKindName::MarkSemiColon])?;

                            Ok(Some(Globals::TypeDef(TypeDef::NativeTypeAlias(
                                NativeTypeAlias {
                                    vis,
                                    ident,
                                    def_id: OnceCell::new(),
                                    genargs,
                                    native,
                                    native_span,
                                    attrs,
                                },
                            ))))
                        } else {
                            Err(ParseError::InvalidEOF {
                                mod_id,
                                expecteds: vec![TkKindName::DslLiteral],
                            })
                        }
                    } else {
                        // "type" <identifier> ( <generic-argument-declaration> )? "=" <type-representation> ";"
                        self.next();

                        let ident = self.consume_identifier()?;

                        let genargs = self.opt_consume_generic_argument_declaration()?;

                        let _ = self.must_consume_next(vec![TkKindName::MarkAssign])?;

                        let right = self.consume_type_representaion()?;

                        let _ = self.must_consume_next(vec![TkKindName::MarkSemiColon])?;

                        Ok(Some(Globals::TypeDef(TypeDef::TypeAlias(TypeAlias {
                            vis,
                            ident,
                            def_id: OnceCell::new(),
                            genargs,
                            right,
                            attrs,
                        }))))
                    }
                }
                TkKind::KwImpl => {
                    // "impl" <generic-argument-declaration>? <type-representation>
                    //        ( ":" <type-representation> )? "{" ... "}"
                    let begin = t.span.clone();
                    self.next();

                    let genargs_decl = self.opt_consume_generic_argument_declaration()?;

                    let self_typ = self.consume_type_representaion()?;

                    // impl ブロックそのものは可視性を持たない
                    // (項目が個別に持つ。trait impl の項目は trait と同じ)。
                    reject_visibility(&vis, "an impl block (put it on each item instead)")?;

                    // `impl Nyoee: Gyao { .. }`
                    //
                    // 直後が `{` か `:` かの 1 トークンで決まるので曖昧さは無い。
                    let trait_typ = if self
                        .consume_next_if_match(vec![TkKindName::MarkColon])
                        .is_some()
                    {
                        Some(self.consume_type_representaion()?)
                    } else {
                        None
                    };

                    self.must_consume_next(vec![TkKindName::MarkLBrace])?;

                    let mut assoc_fns = vec![];
                    let mut methods = vec![];
                    let mut native_assoc_fns = vec![];
                    let mut native_methods = vec![];

                    loop {
                        if let Some(t) = self.peek().copied()
                            && TkKind::MarkRBrace == t.kind
                        {
                            let end = t.span.clone();
                            self.next();

                            return Ok(Some(Globals::ImplBlock(ImplBlock {
                                impl_id: OnceCell::new(),
                                assoc_fns,
                                methods,
                                native_assoc_fns,
                                native_methods,
                                genargs_decl,
                                self_typ,
                                trait_typ,
                                span: Span::merge(&begin, &end),
                            })));
                        } else {
                            let attrs = self.consume_attributes()?;
                            let item_vis = self.opt_consume_visibility()?;
                            if trait_typ.is_some() {
                                // trait impl の項目は trait と同じ可視性になる (Rust と同じ)。
                                reject_visibility(
                                    &item_vis,
                                    "an item of a trait impl (it has the same visibility as the trait)",
                                )?;
                            }
                            let f = self.consume_function_or_method_definition(attrs, item_vis)?;

                            match f {
                                FnOrMethod::Fn(CodeOrNative::Code(f)) => {
                                    assoc_fns.push(f);
                                }
                                FnOrMethod::Fn(CodeOrNative::Native(f)) => {
                                    native_assoc_fns.push(f);
                                }
                                FnOrMethod::Method(CodeOrNative::Code(f)) => {
                                    methods.push(f);
                                }
                                FnOrMethod::Method(CodeOrNative::Native(f)) => {
                                    native_methods.push(f);
                                }
                            }
                        }
                    }
                }
                TkKind::KwTrait => {
                    // "trait" <identifier> <generic-argument-declaration>?
                    //         "{" ( <fn-signature> ";" )* "}"
                    let begin = t.span.clone();
                    self.next();

                    let id = self.consume_identifier()?;
                    let genargs = self.opt_consume_generic_argument_declaration()?;

                    self.must_consume_next(vec![TkKindName::MarkLBrace])?;

                    let mut items = Vec::new();

                    let end = loop {
                        let t = self.peek().ok_or(ParseError::InvalidEOF {
                            mod_id,
                            expecteds: vec![TkKindName::MarkRBrace],
                        })?;
                        if let TkKind::MarkRBrace = t.kind {
                            let end = t.span.clone();
                            self.next();
                            break end;
                        }

                        items.push(self.consume_trait_item()?);
                    };

                    Ok(Some(Globals::TraitDef(TraitDef {
                        vis,
                        id,
                        def_id: OnceCell::new(),
                        self_gen: OnceCell::new(),
                        items,
                        genargs,
                        attrs,
                        span: Span::merge(&begin, &end),
                    })))
                }
                TkKind::DslLiteral(str) => {
                    reject_visibility(&vis, "a native code block")?;
                    let native = str.to_string();
                    let native_span = t.span.clone();
                    self.next();

                    Ok(Some(Globals::NativeCode(NativeCode {
                        native,
                        native_span,
                        attrs,
                    })))
                }
                TkKind::KwScene => {
                    let begin = t.span.clone();
                    self.next();

                    let id = self.consume_identifier()?;

                    let args = self.consume_argsdec()?;

                    let rtype = if self
                        .consume_next_if_match(vec![TkKindName::MarkArrow])
                        .is_some()
                    {
                        RetTypRepr::Typ(self.consume_type_representaion()?)
                    } else {
                        RetTypRepr::Void(Span::new(
                            args.span.module(),
                            args.span.end(),
                            args.span.end(),
                        ))
                    };

                    let t = self.next().ok_or(ParseError::InvalidEOF {
                        mod_id,
                        expecteds: vec![TkKindName::DslLiteral],
                    })?;
                    let end = t.span.clone();
                    if let TkKind::DslLiteral(str) = t.kind {
                        let novel_stmts = biwac_novel_parser::NovelSourceStream::new(
                            str,
                            t.span.clone(),
                            self.interner,
                        )
                        .parse()
                        .map_err(ParseError::NovelParseError)?;

                        Ok(Some(Globals::NovelScene(NovelScene {
                            vis,
                            id,
                            def_id: OnceCell::new(),
                            args,
                            rtype,
                            stmts: novel_stmts,
                            span: Span::merge(&begin, &end),
                            attrs,
                        })))
                    } else {
                        Err(ParseError::InvalidEOF {
                            mod_id,
                            expecteds: vec![TkKindName::DslLiteral],
                        })
                    }
                }
                _ => Err(ParseError::InvalidToken {
                    expecteds: vec![
                        TkKindName::KwMod,
                        TkKindName::KwFn,
                        TkKindName::KwStruct,
                        TkKindName::KwEnum,
                        TkKindName::KwType,
                        TkKindName::KwImport,
                        TkKindName::KwImpl,
                        TkKindName::KwTrait,
                        TkKindName::KwScene,
                    ],
                    found: t.to_owned().clone(),
                }),
            }
        } else {
            Ok(None)
        }
    }

    /// 可視性 (`pub` / `pub(super)` / `pub(package)`) があれば読む。無ければ `Private`。
    ///
    /// <visibility> ::= "pub" ( "(" ( "super" | "package" ) ")" )?
    pub(crate) fn opt_consume_visibility(&mut self) -> Result<Visibility, ParseError<'src>> {
        let Some(begin) = self
            .consume_next_if_match(vec![TkKindName::KwPub])
            .map(|t| t.span.clone())
        else {
            return Ok(Visibility::Private);
        };

        if self
            .consume_next_if_match(vec![TkKindName::MarkLPare])
            .is_none()
        {
            return Ok(Visibility::Public(begin));
        }

        let scope = self.must_consume_next(vec![TkKindName::KwSuper, TkKindName::KwPackage])?;
        let is_super = matches!(scope.kind, TkKind::KwSuper);
        let end = self
            .must_consume_next(vec![TkKindName::MarkRPare])?
            .span
            .clone();
        let span = Span::merge(&begin, &end);

        Ok(if is_super {
            Visibility::Super(span)
        } else {
            Visibility::Package(span)
        })
    }

    /// `{ Red, Rgb(Int, Int), Named { x: Int }, }`
    ///
    /// 宣言順がそのままタグの値になるので、並べ替えずに返す。
    fn consume_variant_declarations(&mut self) -> Result<Vec<VariantDecl>, ParseError<'src>> {
        let mod_id = self.mod_id;
        let _ = self.must_consume_next(vec![TkKindName::MarkLBrace])?;

        let mut variants = Vec::new();

        loop {
            let t = self.peek().ok_or(ParseError::InvalidEOF {
                mod_id,
                expecteds: vec![TkKindName::Ident, TkKindName::MarkRBrace],
            })?;

            if let TkKind::MarkRBrace = t.kind {
                self.next();
                return Ok(variants);
            }

            // variant は常に enum と同じ可視性になる (Rust と同じ)。
            let vis = self.opt_consume_visibility()?;
            reject_visibility(
                &vis,
                "an enum variant (it has the same visibility as the enum)",
            )?;

            let id = self.consume_identifier()?;
            let begin = id.span.clone();

            let (fields, end) = match self.peek().map(|t| &t.kind) {
                // タプル形式。中身は型の並び。
                Some(TkKind::MarkLPare) => {
                    let (typs, span) = self.consume_variant_tuple_fields()?;
                    (VariantFieldsDecl::Tuple(typs), span)
                }
                // 構造体形式。メンバ宣言と同じ書き方をする。
                Some(TkKind::MarkLBrace) => {
                    let (members, span) = self.consume_variant_struct_fields()?;
                    (VariantFieldsDecl::Struct(members), span)
                }
                _ => (VariantFieldsDecl::Unit, begin.clone()),
            };

            variants.push(VariantDecl {
                id,
                def_id: OnceCell::new(),
                fields,
                span: Span::merge(&begin, &end),
            });

            let t = self.must_consume_next(vec![TkKindName::MarkComma, TkKindName::MarkRBrace])?;
            if let TkKind::MarkRBrace = t.kind {
                return Ok(variants);
            }
        }
    }

    fn consume_variant_tuple_fields(
        &mut self,
    ) -> Result<(Vec<(Ident, TypRepr)>, Span), ParseError<'src>> {
        let mod_id = self.mod_id;
        let begin = self
            .must_consume_next(vec![TkKindName::MarkLPare])?
            .span
            .clone();

        let mut typs = Vec::new();

        loop {
            let t = self.peek().ok_or(ParseError::InvalidEOF {
                mod_id,
                expecteds: vec![TkKindName::MarkRPare],
            })?;

            if let TkKind::MarkRPare = t.kind {
                let end = t.span.clone();
                self.next();
                return Ok((typs, Span::merge(&begin, &end)));
            }

            let vis = self.opt_consume_visibility()?;
            reject_visibility(
                &vis,
                "a field of an enum variant (it has the same visibility as the enum)",
            )?;
            let typ = self.consume_type_representaion()?;
            let name = self.interner.get_or_insert(&format!("_{}", typs.len()));
            typs.push((
                Ident {
                    id: name,
                    // 名前はソースに書かれていないので、型の位置を借りる。
                    span: typ.span.clone(),
                },
                typ,
            ));

            let t = self.must_consume_next(vec![TkKindName::MarkComma, TkKindName::MarkRPare])?;
            if let TkKind::MarkRPare = t.kind {
                let end = t.span.clone();
                return Ok((typs, Span::merge(&begin, &end)));
            }
        }
    }

    fn consume_variant_struct_fields(
        &mut self,
    ) -> Result<(Vec<(Ident, TypRepr)>, Span), ParseError<'src>> {
        let mod_id = self.mod_id;
        let begin = self
            .must_consume_next(vec![TkKindName::MarkLBrace])?
            .span
            .clone();

        let mut members = Vec::new();

        loop {
            let t = self.peek().ok_or(ParseError::InvalidEOF {
                mod_id,
                expecteds: vec![TkKindName::Ident, TkKindName::MarkRBrace],
            })?;

            if let TkKind::MarkRBrace = t.kind {
                let end = t.span.clone();
                self.next();
                return Ok((members, Span::merge(&begin, &end)));
            }

            let vis = self.opt_consume_visibility()?;
            reject_visibility(
                &vis,
                "a field of an enum variant (it has the same visibility as the enum)",
            )?;
            let id = self.consume_identifier()?;
            let typ = self.must_consume_type_annotation()?;
            members.push((id, typ));

            let t = self.must_consume_next(vec![TkKindName::MarkComma, TkKindName::MarkRBrace])?;
            if let TkKind::MarkRBrace = t.kind {
                let end = t.span.clone();
                return Ok((members, Span::merge(&begin, &end)));
            }
        }
    }

    pub(crate) fn consume_argsdec(&mut self) -> Result<ArgDeclList, ParseError<'src>> {
        let mod_id = self.mod_id;
        let begin = self
            .must_consume_next(vec![TkKindName::MarkLPare])?
            .span
            .clone();

        let mut args = vec![];

        loop {
            let t = self
                .next()
                .ok_or(ParseError::InvalidEOF {
                    mod_id,
                    expecteds: vec![TkKindName::MarkRPare, TkKindName::Ident],
                })?
                .clone();

            if let TkKind::MarkRPare = t.kind {
                return Ok(ArgDeclList {
                    args,
                    span: Span::merge(&begin, &t.span),
                });
            } else if let TkKind::Ident(arg) = &t.kind {
                let typ = self.must_consume_type_annotation()?;

                args.push(ArgDecl {
                    span: Span::merge(&t.span, &typ.span),
                    typ,
                    id: Ident {
                        id: *arg,
                        span: t.span.clone(),
                    },
                    var_id: OnceCell::new(),
                });

                if let Some(t) = self.peek() {
                    if let TkKind::MarkComma = t.kind {
                        self.next();
                    } else if let TkKind::MarkRPare = t.kind {
                        continue;
                    } else {
                        return Err(ParseError::InvalidToken {
                            expecteds: vec![TkKindName::MarkComma, TkKindName::MarkRPare],
                            found: t.to_owned().clone(),
                        });
                    }
                } else {
                    return Err(ParseError::InvalidEOF {
                        mod_id,
                        expecteds: vec![TkKindName::MarkComma, TkKindName::MarkRPare],
                    });
                }
            }
        }
    }

    fn consume_method_argsdec(
        &mut self,
    ) -> Result<FnOrMethod<ArgDeclList, MethodArgDeclList>, ParseError<'src>> {
        let mod_id = self.mod_id;

        // (args, self_ident)
        let begin = self
            .must_consume_next(vec![TkKindName::MarkLPare])?
            .span
            .clone();

        let mut args = vec![];

        // first arg `self` or not
        let t = self.peek().ok_or(ParseError::InvalidEOF {
            mod_id,
            expecteds: vec![
                TkKindName::MarkRPare,
                TkKindName::Ident,
                TkKindName::KwSelfVar,
            ],
        })?;

        let opt_self_span = match t.kind {
            TkKind::MarkRPare => {
                let end = t.span.clone();
                self.next();

                return Ok(FnOrMethod::Fn(ArgDeclList {
                    args,
                    span: Span::merge(&begin, &end),
                }));
            }
            TkKind::KwSelfVar => {
                let span = t.span.clone();
                self.next();

                Some(span)
            }
            _ => None,
        };

        loop {
            let t = self
                .next()
                .ok_or(ParseError::InvalidEOF {
                    mod_id,
                    expecteds: vec![TkKindName::MarkRPare, TkKindName::Ident],
                })?
                .clone();

            if let TkKind::MarkRPare = t.kind {
                match opt_self_span {
                    Some(self_span) => {
                        return Ok(FnOrMethod::Method(MethodArgDeclList {
                            self_span,
                            args,
                            span: Span::merge(&begin, &t.span),
                        }));
                    }
                    None => {
                        return Ok(FnOrMethod::Fn(ArgDeclList {
                            args,
                            span: Span::merge(&begin, &t.span),
                        }));
                    }
                }
            } else if let TkKind::Ident(arg) = &t.kind {
                let typ = self.must_consume_type_annotation()?;

                args.push(ArgDecl {
                    span: Span::merge(&t.span, &typ.span),
                    typ,
                    id: Ident {
                        id: *arg,
                        span: t.span.clone(),
                    },
                    var_id: OnceCell::new(),
                });

                if let Some(t) = self.peek() {
                    if let TkKind::MarkComma = t.kind {
                        self.next();
                    } else if let TkKind::MarkRPare = t.kind {
                        continue;
                    } else {
                        return Err(ParseError::InvalidToken {
                            expecteds: vec![TkKindName::MarkComma, TkKindName::MarkRPare],
                            found: t.to_owned().clone(),
                        });
                    }
                } else {
                    return Err(ParseError::InvalidEOF {
                        mod_id,
                        expecteds: vec![TkKindName::MarkComma, TkKindName::MarkRPare],
                    });
                }
            }
        }
    }
}

/// 可視性を書けない場所に書かれていればエラーにする。
fn reject_visibility<'src>(vis: &Visibility, place: &'static str) -> Result<(), ParseError<'src>> {
    match vis.span() {
        Some(span) => Err(ParseError::NotAllowedHere {
            span: span.clone(),
            what: "a visibility",
            place,
        }),
        None => Ok(()),
    }
}
