//! 無名関数の持ち上げ。
//!
//! 無名関数は外側の関数と同じ文脈で型推論する (注釈の無い引数・戻り値の型を、
//! 渡した先などの文脈から決めるため)。推論が終わったら、決まった型で
//! トップレベルの関数に持ち上げる。式の位置は [`Lambda::lifted`] を通して
//! その関数への参照になり、後段 (MIR・単相化・codegen) は名前付きの関数の値と同じに扱う。
//!
//! 持ち上げた関数は外側の定義のジェネリック引数と制限をそのまま引き継ぐ
//! (`fn foo[T](x: T) { let g = fn(y: T) { .. }; }` の `g` は `T` について多相になる)。
//! 外側の関数の実体ごとに、持ち上げた関数の実体ができる。

use std::collections::HashMap;

use biwac_base::PackageId;
use biwac_hir::{
    BlockExpr, BlockStmt, DeclaredVisibility, Expr, ExprId, ExprVal, FnArgDecl, FnBody, FnDef,
    FnSignature, Ident, Lambda, Literal, Primary, Stmt, Ty, TyKind, ValDefKind, VariantCtorFields,
    Visibility,
};
use biwac_span::{DefId, LocalGenDefId, PackageLocalDefId, ValDefId, VarId};

use crate::TyCtx;

/// 持ち上げに要る、外側の関数の情報。
struct Enclosing<'h> {
    signature: &'h FnSignature,
    body: &'h FnBody,
    expr_tys: &'h HashMap<ExprId, Ty>,
    var_tys: &'h HashMap<VarId, Ty>,
    call_genargs: &'h HashMap<ExprId, Vec<(LocalGenDefId, Ty)>>,
}

impl TyCtx<'_> {
    /// すべての関数の本体にある無名関数を、トップレベルの関数に持ち上げる。
    ///
    /// 型推論の結果を書き込んだ後に呼ぶ。
    pub(super) fn lift_lambdas(&mut self) {
        let (lifted, next) = self.collect_lifted();
        self.hir.next_def_id = next;
        for (def_id, f) in lifted {
            self.hir.lambdas.insert(def_id);
            self.hir.vals.insert(def_id, ValDefKind::Fn(Box::new(f)));
        }
    }

    /// 持ち上げた関数を作る。`hir` は読むだけで、書き込みは呼ぶ側が行う。
    fn collect_lifted(&self) -> (Vec<(ValDefId, FnDef)>, u32) {
        // 番号の振り方をビルドごとに揃えるため、外側の関数を `ValDefId` 順に辿る。
        let mut enclosings: Vec<(ValDefId, Enclosing)> = Vec::new();
        for (def_id, val) in &self.hir.vals {
            match val {
                ValDefKind::Fn(f) => enclosings.push((*def_id, enclosing_of_fn(f))),
                ValDefKind::NovelScene(s) => enclosings.push((
                    *def_id,
                    Enclosing {
                        signature: &s.signature,
                        body: &s.body,
                        expr_tys: &s.expr_tys,
                        var_tys: &s.var_tys,
                        call_genargs: &s.call_genargs,
                    },
                )),
                ValDefKind::Native(_) => {}
            }
        }
        for ty_impl in self.hir.tys.values() {
            for list in ty_impl.vals.values() {
                for (def_id, pair) in &list.vals {
                    if let biwac_hir::AssocValDefKind::Fn(f) = &pair.val_content {
                        enclosings.push((*def_id, enclosing_of_fn(f)));
                    }
                }
            }
        }
        enclosings.sort_by_key(|(def_id, _)| def_id.value());

        // 1 周目: 番号を振る。入れ子の無名関数も先に振っておく
        // (2 周目で外側の無名関数の本体を複製するとき、内側の番号が入っている必要がある)。
        let mut next = self.hir.next_def_id;
        for (_, enc) in &enclosings {
            walk_body(enc.body, &mut |_, lambda| {
                let def_id = ValDefId::new(DefId::new_in_self_pkg(PackageLocalDefId::new(next)));
                next += 1;
                lambda
                    .lifted
                    .set(def_id)
                    .expect("compiler bug: a lambda is lifted twice");
            });
        }

        // 2 周目: 持ち上げた関数を作る。
        let mut lifted: Vec<(ValDefId, FnDef)> = Vec::new();
        for (_, enc) in &enclosings {
            walk_body(enc.body, &mut |expr_id, lambda| {
                let def_id = *lambda.lifted.get().unwrap();
                lifted.push((def_id, self.lift(def_id, expr_id, lambda, enc)));
            });
        }

        (lifted, next)
    }

    fn lift(&self, def_id: ValDefId, expr_id: ExprId, lambda: &Lambda, enc: &Enclosing) -> FnDef {
        // 無名関数の式の型 (決まった関数型) から、引数と戻り値の型を取る。
        let Some(TyKind::Fn(fty)) = enc.expr_tys.get(&expr_id).map(|t| &t.kind) else {
            panic!("compiler bug: a lambda is not typed as a function after inference")
        };

        let args = lambda
            .args
            .iter()
            .zip(&fty.args)
            .map(|(arg, ty)| FnArgDecl {
                id: arg.id.clone(),
                ty: ty.clone(),
                var_id: arg.var_id,
            })
            .collect();

        // 名前は識別子として正しく (TypeScript の関数名にもなる)、利用者の名前と区別できるものにする。
        // 名前で引けないことは `.biwameta` がモジュールの子に載せないことで保証する。
        let name_id = self
            .interner
            .borrow_mut()
            .get_or_insert(&format!("__lambda{}", def_id.value()));

        let signature = FnSignature {
            args,
            has_self: false,
            impl_self_ty: None,
            rty: (*fty.rty).clone(),
            // 外側の定義のジェネリック引数と制限をそのまま引き継ぐ。
            genargs: enc.signature.genargs.clone(),
            impl_genargs: enc.signature.impl_genargs.clone(),
            span: lambda.span.clone(),
        };

        let body = FnBody {
            stmts: lambda.stmts.clone(),
            expr: lambda.expr.as_deref().cloned(),
            self_var_id: None,
            // 外側と変数の番号が通しなので、外側の表をそのまま使える (余分な分は使われない)。
            vars: enc.body.vars.clone(),
        };

        // 名前で引けないので可視性は意味を持たないが、何も書かなかった関数と同じく
        // 書いたモジュールの中に限っておく。
        let vis = Visibility::resolve(
            DeclaredVisibility::Private,
            PackageId::SELF_PACKAGE,
            lambda.span.module(),
            None,
        )
        .expect("private always resolves");
        let mut f = FnDef::new(
            Ident {
                id: name_id,
                span: lambda.span.clone(),
            },
            vis,
            signature,
            body,
        );
        // 型推論の結果も外側のものを引き継ぐ (式・変数の番号は外側と通し)。
        f.expr_tys = enc.expr_tys.clone();
        f.var_tys = enc.var_tys.clone();
        f.call_genargs = enc.call_genargs.clone();
        f
    }
}

fn enclosing_of_fn(f: &FnDef) -> Enclosing<'_> {
    Enclosing {
        signature: &f.signature,
        body: &f.body,
        expr_tys: &f.expr_tys,
        var_tys: &f.var_tys,
        call_genargs: &f.call_genargs,
    }
}

// ---- HIR の走査 (無名関数を外側から順に見つける) ----

fn walk_body(body: &FnBody, f: &mut impl FnMut(ExprId, &Lambda)) {
    for stmt in &body.stmts {
        walk_stmt(stmt, f);
    }
    if let Some(expr) = &body.expr {
        walk_expr(expr, f);
    }
}

fn walk_stmt(stmt: &Stmt, f: &mut impl FnMut(ExprId, &Lambda)) {
    match stmt {
        Stmt::Block(b) => walk_block_stmt(b, f),
        Stmt::Expr(e) => walk_expr(&e.expr, f),
        Stmt::Return(r) => walk_expr(&r.expr, f),
        Stmt::If(i) => {
            walk_expr(&i.cond, f);
            walk_block_stmt(&i.then, f);
            if let Some(els) = &i.els {
                walk_block_stmt(els, f);
            }
        }
        Stmt::While(w) => {
            walk_expr(&w.cond, f);
            walk_block_stmt(&w.stmts, f);
        }
        Stmt::Match(m) => {
            walk_expr(&m.scrutinee, f);
            for arm in &m.arms {
                walk_block_stmt(&arm.body, f);
            }
        }
        Stmt::VarDecl(v) => walk_expr(&v.init, f),
        Stmt::Assign(a) => {
            walk_primary(None, &a.dst, f);
            walk_expr(&a.src, f);
        }
        Stmt::NovelSyscall(s) => walk_expr(&s.call, f),
    }
}

fn walk_block_stmt(block: &BlockStmt, f: &mut impl FnMut(ExprId, &Lambda)) {
    for stmt in &block.stmts {
        walk_stmt(stmt, f);
    }
}

fn walk_block_expr(block: &BlockExpr, f: &mut impl FnMut(ExprId, &Lambda)) {
    for stmt in &block.stmts {
        walk_stmt(stmt, f);
    }
    walk_expr(&block.expr, f);
}

fn walk_expr(expr: &Expr, f: &mut impl FnMut(ExprId, &Lambda)) {
    match &expr.expr {
        ExprVal::Primary(p) => walk_primary(Some(expr.id), p, f),
        ExprVal::Unary(u) => walk_expr(&u.right, f),
        ExprVal::Binary(b) => {
            walk_expr(&b.left, f);
            walk_expr(&b.right, f);
        }
    }
}

fn walk_primary(expr_id: Option<ExprId>, primary: &Primary, f: &mut impl FnMut(ExprId, &Lambda)) {
    match primary {
        Primary::Literal(Literal::Struct(s)) => {
            for (_, member) in &s.members {
                walk_expr(member, f);
            }
        }
        Primary::Literal(_) | Primary::Variable(_) => {}
        Primary::MemberAccess(m) => walk_expr(&m.left, f),
        Primary::IfExpr(i) => {
            walk_expr(&i.cond, f);
            walk_block_expr(&i.then, f);
            walk_block_expr(&i.els, f);
        }
        Primary::Match(m) => {
            walk_expr(&m.scrutinee, f);
            for arm in &m.arms {
                walk_block_expr(&arm.body, f);
            }
        }
        Primary::Block(b) => walk_block_expr(b, f),
        Primary::Call(c) => {
            walk_expr(&c.callee, f);
            for arg in &c.args {
                walk_expr(arg, f);
            }
        }
        Primary::VariantCtor(v) => match &v.fields {
            VariantCtorFields::Unit => {}
            VariantCtorFields::Positional(exprs) => {
                for e in exprs {
                    walk_expr(e, f);
                }
            }
            VariantCtorFields::Named(fields) => {
                for (_, e) in fields {
                    walk_expr(e, f);
                }
            }
        },
        // 外側から先に見つける (入れ子の無名関数はその後)。
        Primary::Lambda(l) => {
            let expr_id = expr_id.expect("compiler bug: a lambda outside an expression");
            f(expr_id, l);
            for stmt in &l.stmts {
                walk_stmt(stmt, f);
            }
            if let Some(e) = &l.expr {
                walk_expr(e, f);
            }
        }
    }
}
