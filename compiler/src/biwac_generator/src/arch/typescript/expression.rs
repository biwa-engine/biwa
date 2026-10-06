use std::cell::Cell;

use biwac_ast::{BinOperator, UnOperator};
use biwac_hir::{BlockExpr, CallTarget, Expr, ExprVal, Literal, MethodTarget, Primary, VarIdKind};
use biwac_span::VarId;
use oxc_allocator::FromIn;

use crate::arch::typescript::{AsOxcLocal, Mangled, span, yield_expr};

impl Mangled for VarIdKind {
    fn mangled(&self, ctx: &super::AstBuildCtx) -> String {
        match self {
            // 呼び先は関数そのものなので、self 型は名前に出ない。
            VarIdKind::Fn(def_id) | VarIdKind::Assoc { def_id, .. } => {
                ctx.get_value_mangled(def_id)
            }
            // 実装が単相化まで決まらない項目は TypeScript では出せない。
            // 呼び出しは driver がこの手前で弾き、値にするのは型推論が弾いている。
            VarIdKind::TraitAssoc { .. } => panic!(
                "compiler bug: a trait-bound item reached the TypeScript backend; \
                 the driver must reject it first"
            ),
            VarIdKind::Local(var_id) => var_id.mangled(ctx),
        }
    }
}

impl Mangled for VarId {
    fn mangled(&self, _ctx: &super::AstBuildCtx) -> String {
        format!("__lv{}", self.value())
    }
}

impl<'a> AsOxcLocal<'a, oxc_ast::ast::Expression<'a>> for BlockExpr {
    fn as_oxc_local(
        &'a self,
        ctx: &'a super::AstBuildCtx<'a>,
        fctx: &mut super::FnAstBuildCtx<'a>,
    ) -> oxc_ast::ast::Expression<'a> {
        let oxc_stmts = self
            .stmts
            .iter()
            .map(|stmt| stmt.as_oxc_local(ctx, fctx))
            .collect::<Vec<_>>();

        fctx.stmts.extend(oxc_stmts);

        self.expr.as_oxc_local(ctx, fctx)
    }
}

impl<'a> AsOxcLocal<'a, oxc_ast::ast::UnaryOperator> for UnOperator {
    fn as_oxc_local(
        &'a self,
        _ctx: &'a super::AstBuildCtx<'a>,
        _fctx: &mut super::FnAstBuildCtx<'a>,
    ) -> oxc_ast::ast::UnaryOperator {
        match self {
            Self::Neg => oxc_ast::ast::UnaryOperator::UnaryNegation,
        }
    }
}

impl<'a> AsOxcLocal<'a, oxc_ast::ast::BinaryOperator> for BinOperator {
    fn as_oxc_local(
        &'a self,
        _ctx: &'a super::AstBuildCtx<'a>,
        _fctx: &mut super::FnAstBuildCtx<'a>,
    ) -> oxc_ast::ast::BinaryOperator {
        match self {
            Self::Add => oxc_ast::ast::BinaryOperator::Addition, // + TS: "+"
            Self::Sub => oxc_ast::ast::BinaryOperator::Subtraction, // - TS: "-"
            Self::Mul => oxc_ast::ast::BinaryOperator::Multiplication, // * TS: "*"
            Self::Div => oxc_ast::ast::BinaryOperator::Division, // / TS: "/"
            Self::Mod => oxc_ast::ast::BinaryOperator::Remainder, // % TS: "%"
            Self::Gt => oxc_ast::ast::BinaryOperator::GreaterThan, // > TS: ">"
            Self::Lt => oxc_ast::ast::BinaryOperator::LessThan,  // < TS: "<"
            Self::Ge => oxc_ast::ast::BinaryOperator::GreaterEqualThan, // >= TS: ">="
            Self::Le => oxc_ast::ast::BinaryOperator::LessEqualThan, // <= TS: "<="
            Self::Eq => oxc_ast::ast::BinaryOperator::StrictEquality, // == TS: "==="
            Self::Ne => oxc_ast::ast::BinaryOperator::StrictInequality, // != TS: "!=="
        }
    }
}

impl<'a> AsOxcLocal<'a, oxc_ast::ast::Expression<'a>> for Expr {
    fn as_oxc_local(
        &'a self,
        ctx: &'a super::AstBuildCtx<'a>,
        fctx: &mut super::FnAstBuildCtx<'a>,
    ) -> oxc_ast::ast::Expression<'a> {
        match &self.expr {
            ExprVal::Primary(p) => match p {
                Primary::Literal(l) => match l {
                    Literal::Integer(i) => {
                        oxc_ast::ast::Expression::NumericLiteral(oxc_allocator::Box::new_in(
                            oxc_ast::ast::NumericLiteral {
                                span: span(),
                                value: i.val as f64,
                                raw: None,
                                base: oxc_ast::ast::NumberBase::Decimal,
                            },
                            ctx.allocator,
                        ))
                    }
                    // TypeScript には整数と浮動小数点数の区別が無いので、
                    // どちらも `number` のリテラルになる。
                    Literal::Float(f) => {
                        oxc_ast::ast::Expression::NumericLiteral(oxc_allocator::Box::new_in(
                            oxc_ast::ast::NumericLiteral {
                                span: span(),
                                value: f.val,
                                raw: None,
                                base: oxc_ast::ast::NumberBase::Decimal,
                            },
                            ctx.allocator,
                        ))
                    }
                    Literal::Bool(b) => {
                        oxc_ast::ast::Expression::BooleanLiteral(oxc_allocator::Box::new_in(
                            oxc_ast::ast::BooleanLiteral {
                                span: span(),
                                value: b.val,
                            },
                            ctx.allocator,
                        ))
                    }
                    Literal::String(s) => {
                        oxc_ast::ast::Expression::StringLiteral(oxc_allocator::Box::new_in(
                            oxc_ast::ast::StringLiteral {
                                span: span(),
                                value: oxc_ast::ast::Atom::from_in(&s.val, ctx.allocator),
                                raw: None,
                                lone_surrogates: false,
                            },
                            ctx.allocator,
                        ))
                    }
                    Literal::Struct(s) => {
                        oxc_ast::ast::Expression::ObjectExpression(oxc_allocator::Box::new_in(
                            oxc_ast::ast::ObjectExpression {
                                span: span(),
                                properties: oxc_allocator::Vec::from_iter_in(
                                    s.members.iter().map(|(ident, expr)| {
                                        oxc_ast::ast::ObjectPropertyKind::ObjectProperty(
                                            oxc_allocator::Box::new_in(
                                                oxc_ast::ast::ObjectProperty {
                                                    span: span(),
                                                    kind: oxc_ast::ast::PropertyKind::Init,
                                                    key:
                                                        oxc_ast::ast::PropertyKey::StaticIdentifier(
                                                            oxc_allocator::Box::new_in(
                                                                oxc_ast::ast::IdentifierName {
                                                                    span: span(),
                                                                    name:
                                                                        oxc_span::Ident::new_const(
                                                                            ctx.allocator.alloc(
                                                                                ctx.str_of(
                                                                                    &ident.id,
                                                                                ),
                                                                            ),
                                                                        ),
                                                                },
                                                                ctx.allocator,
                                                            ),
                                                        ),
                                                    value: expr.as_oxc_local(ctx, fctx),
                                                    method: false,
                                                    shorthand: false,
                                                    computed: false,
                                                },
                                                ctx.allocator,
                                            ),
                                        )
                                    }),
                                    ctx.allocator,
                                ),
                            },
                            ctx.allocator,
                        ))
                    }
                },
                // 無名関数は持ち上げた関数への参照である (関数は hir.vals の側から出力される)。
                Primary::Lambda(l) => {
                    let def_id = l
                        .lifted
                        .get()
                        .expect("compiler bug: a lambda is not lifted after inference");
                    oxc_ast::ast::Expression::Identifier(oxc_allocator::Box::new_in(
                        oxc_ast::ast::IdentifierReference {
                            span: span(),
                            name: oxc_span::Ident::new_const(
                                ctx.allocator.alloc_str(&ctx.get_value_mangled(def_id)),
                            ),
                            reference_id: Cell::new(None),
                        },
                        ctx.allocator,
                    ))
                }
                Primary::Variable(v) => {
                    oxc_ast::ast::Expression::Identifier(oxc_allocator::Box::new_in(
                        oxc_ast::ast::IdentifierReference {
                            span: span(),
                            name: oxc_span::Ident::new_const(
                                ctx.allocator.alloc_str(&v.id.mangled(ctx)),
                            ),
                            reference_id: Cell::new(None),
                        },
                        ctx.allocator,
                    ))
                }
                Primary::Call(c) => {
                    let target = *c
                        .target
                        .get()
                        .expect("compiler bug: a call is not classified after inference");

                    let arg = |e: &'a Expr, fctx: &mut super::FnAstBuildCtx<'a>| {
                        oxc_ast::ast::Argument::from(e.as_oxc_local(ctx, fctx))
                    };
                    let (callee, args) = match target {
                        // 関数・関連関数は名前、関数型の値は式そのものを呼ぶ。
                        // 関数型のメンバ (`self.on_click(e)`) は、struct がオブジェクトなので
                        // `left.member(args)` がそのままメンバの関数を呼ぶ。
                        CallTarget::Static(_) | CallTarget::Value => {
                            let callee = c.callee.as_oxc_local(ctx, fctx);
                            let args: Vec<_> = c.args.iter().map(|a| arg(a, fctx)).collect();
                            (callee, args)
                        }
                        // メソッドは関数として出力されている。self は第一引数として与える。
                        CallTarget::Method(MethodTarget::Direct(def_id)) => {
                            let ExprVal::Primary(Primary::MemberAccess(m)) = &c.callee.expr else {
                                panic!("compiler bug: a method call without a member access callee")
                            };
                            let callee =
                                oxc_ast::ast::Expression::Identifier(oxc_allocator::Box::new_in(
                                    oxc_ast::ast::IdentifierReference {
                                        span: span(),
                                        name: oxc_span::Ident::new_const(
                                            ctx.allocator
                                                .alloc_str(&ctx.get_value_mangled(&def_id)),
                                        ),
                                        reference_id: Cell::new(None),
                                    },
                                    ctx.allocator,
                                ));
                            let mut args = vec![arg(&m.left, fctx)];
                            args.extend(c.args.iter().map(|a| arg(a, fctx)));
                            (callee, args)
                        }
                        // driver が手前で弾いているので、ここには来ない。
                        CallTarget::Method(MethodTarget::Trait(_)) | CallTarget::TraitItem(_) => {
                            panic!(
                                "compiler bug: a trait-bound call reached the TypeScript backend; \
                                 the driver must reject it first"
                            )
                        }
                    };

                    let call =
                        oxc_ast::ast::Expression::CallExpression(oxc_allocator::Box::new_in(
                            oxc_ast::ast::CallExpression {
                                span: span(),
                                callee,
                                type_arguments: None,
                                arguments: oxc_allocator::Vec::from_iter_in(args, ctx.allocator),
                                optional: false,
                                pure: false,
                            },
                            ctx.allocator,
                        ));

                    // scene は generator なので、呼ぶ側が委譲しなければならない。
                    // 呼び出し先が出した syscall はそのまま外側の kernel まで抜ける。
                    match target {
                        CallTarget::Static(def_id) if ctx.is_scene(&def_id) => {
                            yield_expr(call, true, ctx.allocator)
                        }
                        _ => call,
                    }
                }
                Primary::VariantCtor(ctor) => ctor.as_oxc_local(ctx, fctx),
                Primary::Match(m) => super::enums::match_expr_as_oxc(m, self.id, ctx, fctx),
                Primary::IfExpr(if_expr) => {
                    if if_expr.then.stmts.is_empty() && if_expr.els.stmts.is_empty() {
                        // then と else のブロック式が文を全く含まない場合、
                        // TS三項演算子 ? : を使えば良い
                        oxc_ast::ast::Expression::ConditionalExpression(oxc_allocator::Box::new_in(
                            oxc_ast::ast::ConditionalExpression {
                                span: span(),
                                test: if_expr.cond.as_oxc_local(ctx, fctx),
                                consequent: if_expr.then.as_oxc_local(ctx, fctx),
                                alternate: if_expr.els.as_oxc_local(ctx, fctx),
                            },
                            ctx.allocator,
                        ))
                    } else {
                        // そうでない場合、
                        // let __tmpx;
                        // if (cond) {
                        //   S
                        //   S
                        //   __tmpx = E;
                        // } else {
                        //   S
                        //   __tmpx = E;
                        // }
                        // をenv.stmtsに追加し、
                        // 式の中では
                        // __tmpx
                        // を参照する
                        todo!()
                    }
                }
                Primary::MemberAccess(m) => {
                    oxc_ast::ast::Expression::StaticMemberExpression(oxc_allocator::Box::new_in(
                        oxc_ast::ast::StaticMemberExpression {
                            span: span(),
                            object: m.left.as_oxc_local(ctx, fctx),
                            property: oxc_ast::ast::IdentifierName {
                                span: span(),
                                name: oxc_span::Ident::new_const(
                                    ctx.allocator.alloc(ctx.str_of(&m.member.id)),
                                ),
                            },
                            optional: false,
                        },
                        ctx.allocator,
                    ))
                }
                Primary::Block(block) => block.as_oxc_local(ctx, fctx),
            },
            ExprVal::Unary(u) => {
                oxc_ast::ast::Expression::UnaryExpression(oxc_allocator::Box::new_in(
                    oxc_ast::ast::UnaryExpression {
                        span: span(),
                        operator: u.op.as_oxc_local(ctx, fctx),
                        argument: u.right.as_oxc_local(ctx, fctx),
                    },
                    ctx.allocator,
                ))
            }
            ExprVal::Binary(b) => {
                oxc_ast::ast::Expression::BinaryExpression(oxc_allocator::Box::new_in(
                    oxc_ast::ast::BinaryExpression {
                        span: span(),
                        operator: b.op.as_oxc_local(ctx, fctx),
                        left: b.left.as_oxc_local(ctx, fctx),
                        right: b.right.as_oxc_local(ctx, fctx),
                    },
                    ctx.allocator,
                ))
            }
        }
    }
}
