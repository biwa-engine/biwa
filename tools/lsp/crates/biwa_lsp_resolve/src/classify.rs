//! 名前解決済みの `biwac_ast` から、識別子の使用箇所ごとの意味的な種別を
//! 取り出し、semantic highlighting の上書きに使えるリストへ変換する。
//!
//! # 設計
//!
//! `biwac_name_resolver::NameResolver` は `&mut Pkg` を受け取って
//! `Path::segments` の `resolved_id: OnceCell<PathSegmentResolution>` を
//! その場で埋める。呼び出し側は `try_resolve` が `Ok`/`Err` のどちらであっても
//! 自分が渡した `Pkg` を (途中まで埋まった状態で) そのまま持ち続けられるので、
//! この関数はその `Pkg` の AST を直接読むだけでよい。
//!
//! 以前は `Hir` から分類していたが、`Hir` は複数セグメントパスの
//! 「最後のセグメント以外」の解決結果を保持しないうえ、`FnCall`/`StructLiteral`
//! の呼び出し先・型名の正確な span も持たない (呼び出し式全体の span しかない)。
//! そのため一度 lower し直した AST と span で突き合わせるハックが必要だったが、
//! `Pkg` の AST に直接 `resolved_id` が残るようになったことで、そのハックは
//! 丸ごと不要になった。各 `PathSegment` は `ident.span` (そのセグメント自身の
//! 正確な span) と `resolved_id.get()` (`DefIdKind`) を両方持っているので、
//! 素直に AST を辿るだけで済む。
//!
//! # 既知の制約
//!
//! - enum のバリアント (`DefIdKind::Variant`) に対応する semantic token 種別が
//!   無いので分類しない。
//! - `scene` の本体のうち `@` 行 (キャラクター指定) は `biwa_lsp_lower` が
//!   `NovelStmt` へ変換しない (実コンパイラ自身もまだ持たない文法) ので
//!   分類しようがない。
//! - `Self` 型 (`TypReprVal::SelfTyp`) は `OnceCell` を持たないため分類しない。
//!
//! # メソッド呼び出しの分類について
//!
//! `.foo()` がどのメソッドを指すかは名前解決の範囲外で、型推論
//! (`biwac_type_inferrer`) が `biwac_hir::MethodCall::target` を埋めて
//! 初めて決まる。これは `biwac_ast` 側には対応する field が無い
//! (`biwac_ast::MethodCall` に `target` は無い) ので、`classify` (この
//! ファイルの AST ベースの関数) では扱えない。型推論後に呼ぶ
//! [`classify_resolved_methods`] が `Hir` を直接辿って埋める。
//! `MethodCall::method: Ident` はそのメソッド名の識別子自身の span を
//! そのまま持っている (元の AST の `Ident::from(mc.method.clone())` を
//! 素通ししているだけ) ので、`FnCall`/`StructLiteral` のときのような
//! span 突き合わせは要らない。

use std::collections::HashSet;

use biwac_ast::symbols::globals::GenArgsDecl;
use biwac_ast::{
    AbsolutePathHeader, ArgDeclList, BlockExpr, BlockStmt, Exprs, FnDef, Globals, ImplBlock,
    Literal, MethodArgDeclList, MethodDef, NativeFnDef, NativeMethodDef, NovelContent, NovelScene,
    NovelStmt, Path, PathSegmentResolution, Pattern, PatternFields, Primary, RetTypRepr, Stmt,
    TraitDef, TraitItemArgs, TraitItemDecl, TypDecl, TypRepr, TypReprVal, TypeDef, Variable,
    VariantDecl, VariantFieldsDecl,
};
use biwac_base::ModId;
use biwac_package_loader::{LoadedModule, Pkg};
use biwac_span::{DefIdKind, Span, VarId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedKind {
    Function,
    Method,
    Parameter,
    Type,
    Interface,
    Namespace,
}

pub struct Classification {
    pub start: usize,
    pub end: usize,
    pub kind: ResolvedKind,
}

pub(crate) fn classify(pkg: &Pkg, doc_mod_id: ModId) -> Vec<Classification> {
    let mut out = Vec::new();
    if let Some(module) = find_module(&pkg.root_module, doc_mod_id) {
        for g in &module.ast.globals {
            classify_global(g, &mut out);
        }
    }
    out.sort_by_key(|c| c.start);
    out
}

fn find_module(module: &LoadedModule, mod_id: ModId) -> Option<&LoadedModule> {
    if module.mod_id == mod_id {
        return Some(module);
    }
    module
        .children
        .values()
        .find_map(|c| find_module(c, mod_id))
}

fn push(out: &mut Vec<Classification>, span: Span, kind: ResolvedKind) {
    if !span.is_dummy() {
        out.push(Classification {
            start: span.begin(),
            end: span.end(),
            kind,
        });
    }
}

fn def_id_kind_to_resolved(kind: &DefIdKind, param_ids: &HashSet<VarId>) -> Option<ResolvedKind> {
    match kind {
        DefIdKind::Package(_) | DefIdKind::Mod(_) => Some(ResolvedKind::Namespace),
        DefIdKind::Ty(_) | DefIdKind::Gen(_) | DefIdKind::LocalGen(_) => Some(ResolvedKind::Type),
        DefIdKind::Trait(_) => Some(ResolvedKind::Interface),
        DefIdKind::Val(_) | DefIdKind::TraitAssoc(_) => Some(ResolvedKind::Function),
        DefIdKind::Var(vid) => param_ids.contains(vid).then_some(ResolvedKind::Parameter),
        DefIdKind::Variant(_) => None,
    }
}

fn classify_path(path: &Path, param_ids: &HashSet<VarId>, out: &mut Vec<Classification>) {
    if let Some(AbsolutePathHeader::SelfTyp(self_typ)) = &path.abs_header {
        if self_typ.resolved_id.get().is_some() {
            push(out, self_typ.span.clone(), ResolvedKind::Type);
        }
    }

    for seg in &path.segments {
        if let Some(PathSegmentResolution::Ok(kind)) = seg.resolved_id.get() {
            if let Some(rk) = def_id_kind_to_resolved(kind, param_ids) {
                push(out, seg.ident.span.clone(), rk);
            }
        }
    }
}

// ── 型表現 ──────────────────────────────────────────────────────────────

fn classify_typ_repr(t: &TypRepr, out: &mut Vec<Classification>) {
    match &t.val {
        TypReprVal::Primitive(_) | TypReprVal::SelfTyp => {}
        TypReprVal::Defined(def_typ) => {
            classify_path(&def_typ.path, &HashSet::new(), out);
            if let Some(genargs) = &def_typ.genargs {
                for g in genargs {
                    classify_typ_repr(g, out);
                }
            }
        }
        TypReprVal::Fn(fn_typ) => {
            for a in &fn_typ.args {
                classify_typ_repr(a, out);
            }
            if let Some(rty) = &fn_typ.rty {
                classify_typ_repr(rty, out);
            }
        }
    }
}

fn classify_ret_typ_repr(r: &RetTypRepr, out: &mut Vec<Classification>) {
    if let RetTypRepr::Typ(t) = r {
        classify_typ_repr(t, out);
    }
}

fn classify_typ_decl(t: &TypDecl, out: &mut Vec<Classification>) {
    if let TypDecl::Typ(t) = t {
        classify_typ_repr(t, out);
    }
}

fn classify_genargs_decl<I>(decl: &Option<GenArgsDecl<I>>, out: &mut Vec<Classification>) {
    let Some(decl) = decl else { return };
    for item in &decl.genargs {
        for bound in &item.bounds {
            classify_typ_repr(bound, out);
        }
    }
}

fn classify_arg_decl_list(args: &ArgDeclList, out: &mut Vec<Classification>) {
    for a in &args.args {
        classify_typ_repr(&a.typ, out);
    }
}

fn classify_method_arg_decl_list(args: &MethodArgDeclList, out: &mut Vec<Classification>) {
    for a in &args.args {
        classify_typ_repr(&a.typ, out);
    }
}

// ── グローバル項目 ──────────────────────────────────────────────────────

fn classify_global(g: &Globals, out: &mut Vec<Classification>) {
    match g {
        Globals::Import(i) => classify_path(&i.path, &HashSet::new(), out),
        Globals::FnDef(f) => classify_fn_def(f, out),
        Globals::VarDecl(v) => {
            classify_typ_decl(&v.typ, out);
            classify_exprs(&v.init, &HashSet::new(), out);
        }
        Globals::TypeDef(t) => classify_type_def(t, out),
        Globals::TraitDef(t) => classify_trait_def(t, out),
        Globals::ImplBlock(b) => classify_impl_block(b, out),
        Globals::NativeFnDef(f) => classify_native_fn_def(f, out),
        Globals::NativeCode(_) => {}
        Globals::NovelScene(s) => classify_novel_scene(s, out),
    }
}

fn classify_fn_def(f: &FnDef, out: &mut Vec<Classification>) {
    classify_arg_decl_list(&f.args, out);
    classify_ret_typ_repr(&f.rtype, out);
    classify_genargs_decl(&f.genargs, out);

    let param_ids: HashSet<VarId> = f
        .args
        .args
        .iter()
        .filter_map(|a| a.var_id.get().copied())
        .collect();
    classify_body(&f.stmts, &f.expr, &param_ids, out);
}

fn classify_method_def(m: &MethodDef, out: &mut Vec<Classification>) {
    classify_method_arg_decl_list(&m.args, out);
    classify_ret_typ_repr(&m.rtype, out);
    classify_genargs_decl(&m.genargs, out);

    let param_ids: HashSet<VarId> = m
        .args
        .args
        .iter()
        .filter_map(|a| a.var_id.get().copied())
        .collect();
    classify_body(&m.stmts, &m.expr, &param_ids, out);
}

fn classify_native_fn_def(f: &NativeFnDef, out: &mut Vec<Classification>) {
    classify_arg_decl_list(&f.args, out);
    classify_ret_typ_repr(&f.rtype, out);
    classify_genargs_decl(&f.genargs, out);
}

fn classify_native_method_def(m: &NativeMethodDef, out: &mut Vec<Classification>) {
    classify_method_arg_decl_list(&m.args, out);
    classify_ret_typ_repr(&m.rtype, out);
    classify_genargs_decl(&m.genargs, out);
}

fn classify_impl_block(b: &ImplBlock, out: &mut Vec<Classification>) {
    classify_typ_repr(&b.self_typ, out);
    if let Some(t) = &b.trait_typ {
        classify_typ_repr(t, out);
    }
    classify_genargs_decl(&b.genargs_decl, out);
    for f in &b.assoc_fns {
        classify_fn_def(f, out);
    }
    for m in &b.methods {
        classify_method_def(m, out);
    }
    for f in &b.native_assoc_fns {
        classify_native_fn_def(f, out);
    }
    for m in &b.native_methods {
        classify_native_method_def(m, out);
    }
}

fn classify_type_def(t: &TypeDef, out: &mut Vec<Classification>) {
    match t {
        TypeDef::Struct(s) => {
            for (_, typ) in &s.members {
                classify_typ_repr(typ, out);
            }
            classify_genargs_decl(&s.genargs, out);
        }
        TypeDef::Enum(e) => {
            for v in &e.variants {
                classify_variant_decl(v, out);
            }
            classify_genargs_decl(&e.genargs, out);
        }
        TypeDef::TypeAlias(a) => {
            classify_typ_repr(&a.right, out);
            classify_genargs_decl(&a.genargs, out);
        }
        TypeDef::NativeTypeAlias(na) => {
            classify_genargs_decl(&na.genargs, out);
        }
    }
}

fn classify_variant_decl(v: &VariantDecl, out: &mut Vec<Classification>) {
    match &v.fields {
        VariantFieldsDecl::Unit => {}
        VariantFieldsDecl::Tuple(fields) | VariantFieldsDecl::Struct(fields) => {
            for (_, typ) in fields {
                classify_typ_repr(typ, out);
            }
        }
    }
}

fn classify_trait_def(t: &TraitDef, out: &mut Vec<Classification>) {
    classify_genargs_decl(&t.genargs, out);
    for item in &t.items {
        classify_trait_item_decl(item, out);
    }
}

fn classify_trait_item_decl(item: &TraitItemDecl, out: &mut Vec<Classification>) {
    match &item.args {
        TraitItemArgs::Assoc(list) => classify_arg_decl_list(list, out),
        TraitItemArgs::Method(list) => classify_method_arg_decl_list(list, out),
    }
    classify_ret_typ_repr(&item.rtype, out);
    classify_genargs_decl(&item.genargs, out);
}

fn classify_novel_scene(s: &NovelScene, out: &mut Vec<Classification>) {
    classify_arg_decl_list(&s.args, out);
    classify_ret_typ_repr(&s.rtype, out);

    let param_ids: HashSet<VarId> = s
        .args
        .args
        .iter()
        .filter_map(|a| a.var_id.get().copied())
        .collect();
    classify_novel_stmts(&s.stmts, &param_ids, out);
}

fn classify_novel_stmts(
    stmts: &[NovelStmt],
    param_ids: &HashSet<VarId>,
    out: &mut Vec<Classification>,
) {
    for s in stmts {
        classify_novel_stmt(s, param_ids, out);
    }
}

fn classify_novel_stmt(s: &NovelStmt, param_ids: &HashSet<VarId>, out: &mut Vec<Classification>) {
    match s {
        NovelStmt::Expr(e) => classify_exprs(&e.expr, param_ids, out),
        NovelStmt::If(i) => {
            classify_exprs(&i.cond, param_ids, out);
            classify_novel_stmts(&i.then.stmts, param_ids, out);
            if let Some(els) = &i.els {
                classify_novel_stmts(&els.stmts, param_ids, out);
            }
        }
        NovelStmt::VarDecl(v) => {
            classify_typ_decl(&v.typ, out);
            classify_exprs(&v.init, param_ids, out);
        }
        NovelStmt::Assign(a) => {
            classify_primary(&a.dst, param_ids, out);
            classify_exprs(&a.src, param_ids, out);
        }
        NovelStmt::ContentPush(NovelContent::Expr { expr, .. }) => {
            classify_exprs(expr, param_ids, out);
        }
        NovelStmt::ContentPush(NovelContent::Text { .. }) => {}
        NovelStmt::ContentFlushAndWait(_) => {}
        NovelStmt::NovelEndScene(e) => classify_exprs(&e.expr, param_ids, out),
    }
}

// ── 文・式・パターン ────────────────────────────────────────────────────

fn classify_body(
    stmts: &[Stmt],
    expr: &Option<Exprs>,
    param_ids: &HashSet<VarId>,
    out: &mut Vec<Classification>,
) {
    for s in stmts {
        classify_stmt(s, param_ids, out);
    }
    if let Some(e) = expr {
        classify_exprs(e, param_ids, out);
    }
}

fn classify_stmt(s: &Stmt, param_ids: &HashSet<VarId>, out: &mut Vec<Classification>) {
    match s {
        Stmt::Block(b) => classify_block_stmt(b, param_ids, out),
        Stmt::Expr(e) => classify_exprs(&e.expr, param_ids, out),
        Stmt::Return(r) => classify_exprs(&r.expr, param_ids, out),
        Stmt::If(i) => {
            classify_exprs(&i.cond, param_ids, out);
            classify_block_stmt(&i.then, param_ids, out);
            if let Some(els) = &i.els {
                classify_block_stmt(els, param_ids, out);
            }
        }
        Stmt::Match(m) => {
            classify_exprs(&m.scrutinee, param_ids, out);
            for arm in &m.arms {
                classify_pattern(&arm.pattern, param_ids, out);
                classify_block_stmt(&arm.body, param_ids, out);
            }
        }
        Stmt::While(w) => {
            classify_exprs(&w.cond, param_ids, out);
            classify_block_stmt(&w.stmts, param_ids, out);
        }
        Stmt::VarDecl(v) => {
            classify_typ_decl(&v.typ, out);
            classify_exprs(&v.init, param_ids, out);
        }
        Stmt::Assign(a) => {
            classify_primary(&a.dst, param_ids, out);
            classify_exprs(&a.src, param_ids, out);
        }
    }
}

fn classify_block_stmt(b: &BlockStmt, param_ids: &HashSet<VarId>, out: &mut Vec<Classification>) {
    for s in &b.stmts {
        classify_stmt(s, param_ids, out);
    }
}

fn classify_exprs(e: &Exprs, param_ids: &HashSet<VarId>, out: &mut Vec<Classification>) {
    match e {
        Exprs::Primary(p) => classify_primary(p, param_ids, out),
        Exprs::Unary(u) => classify_exprs(&u.right, param_ids, out),
        Exprs::Binary(b) => {
            classify_exprs(&b.left, param_ids, out);
            classify_exprs(&b.right, param_ids, out);
        }
    }
}

fn classify_primary(p: &Primary, param_ids: &HashSet<VarId>, out: &mut Vec<Classification>) {
    match p {
        Primary::Literal(Literal::Struct(s)) => {
            classify_path(&s.path, param_ids, out);
            for (_, e) in &s.members {
                classify_exprs(e, param_ids, out);
            }
        }
        Primary::Literal(_) => {}
        Primary::Variable(Variable::Path(path)) => classify_path(path, param_ids, out),
        Primary::Variable(Variable::SelfVar(span)) => {
            push(out, span.clone(), ResolvedKind::Parameter);
        }
        Primary::FnCall(f) => {
            classify_path(&f.path, param_ids, out);
            for a in &f.args {
                classify_exprs(a, param_ids, out);
            }
        }
        Primary::MemberAccess(m) => classify_exprs(&m.left, param_ids, out),
        Primary::MethodCall(m) => {
            classify_exprs(&m.left, param_ids, out);
            for a in &m.args {
                classify_exprs(a, param_ids, out);
            }
        }
        Primary::IfExpr(i) => {
            classify_exprs(&i.cond, param_ids, out);
            classify_block_expr(&i.then, param_ids, out);
            classify_block_expr(&i.els, param_ids, out);
        }
        Primary::Match(m) => {
            classify_exprs(&m.scrutinee, param_ids, out);
            for arm in &m.arms {
                classify_pattern(&arm.pattern, param_ids, out);
                classify_block_expr(&arm.body, param_ids, out);
            }
        }
        Primary::Block(b) => classify_block_expr(b, param_ids, out),
    }
}

fn classify_block_expr(b: &BlockExpr, param_ids: &HashSet<VarId>, out: &mut Vec<Classification>) {
    for s in &b.stmts {
        classify_stmt(s, param_ids, out);
    }
    classify_exprs(&b.expr, param_ids, out);
}

fn classify_pattern(p: &Pattern, param_ids: &HashSet<VarId>, out: &mut Vec<Classification>) {
    match p {
        Pattern::Wildcard(_) => {}
        // 単独の識別子。unit バリアントに解決されていれば `resolved_id` が
        // 埋まる。束縛ならどこにも解決されないので何も出ない。
        Pattern::Ident(ip) => classify_path(&ip.path, param_ids, out),
        Pattern::Variant(vp) => {
            classify_path(&vp.path, param_ids, out);
            match &vp.fields {
                PatternFields::Unit => {}
                PatternFields::Tuple(pats) => {
                    for pp in pats {
                        classify_pattern(pp, param_ids, out);
                    }
                }
                PatternFields::Struct(fields) => {
                    for (_, pp) in fields {
                        classify_pattern(pp, param_ids, out);
                    }
                }
            }
        }
    }
}

// ── メソッド呼び出し (型推論後の Hir を辿る) ──────────────────────────────

/// 型推論が解決したメソッド呼び出しを `Method` として分類する。
/// `hir` は `biwac_type_inferrer::TyCtx::infer()` が返した (= `OnceCell` が
/// 埋まった) ものでなければならない。
pub(crate) fn classify_resolved_methods(
    hir: &biwac_hir::Hir,
    doc_mod_id: ModId,
) -> Vec<Classification> {
    let mut out = Vec::new();

    for val in hir.vals.values() {
        match val {
            biwac_hir::ValDefKind::Fn(f) if f.name.span.module() == doc_mod_id => {
                walk_hir_body(&f.body, doc_mod_id, &mut out);
            }
            biwac_hir::ValDefKind::NovelScene(s) if s.name.span.module() == doc_mod_id => {
                walk_hir_body(&s.body, doc_mod_id, &mut out);
            }
            _ => {}
        }
    }

    for ty_impl in hir.tys.values() {
        for list in ty_impl.vals.values() {
            for pair in list.vals.values() {
                if let biwac_hir::AssocValDefKind::Fn(f) = &pair.val_content
                    && f.name.span.module() == doc_mod_id
                {
                    walk_hir_body(&f.body, doc_mod_id, &mut out);
                }
            }
        }
    }

    out
}

fn walk_hir_body(body: &biwac_hir::FnBody, doc_mod_id: ModId, out: &mut Vec<Classification>) {
    for s in &body.stmts {
        walk_hir_stmt(s, doc_mod_id, out);
    }
    if let Some(e) = &body.expr {
        walk_hir_expr(e, doc_mod_id, out);
    }
}

fn walk_hir_stmt(s: &biwac_hir::Stmt, doc_mod_id: ModId, out: &mut Vec<Classification>) {
    use biwac_hir::Stmt as S;
    match s {
        S::Block(b) => walk_hir_block_stmt(b, doc_mod_id, out),
        S::Expr(e) => walk_hir_expr(&e.expr, doc_mod_id, out),
        S::Return(r) => walk_hir_expr(&r.expr, doc_mod_id, out),
        S::If(i) => {
            walk_hir_expr(&i.cond, doc_mod_id, out);
            walk_hir_block_stmt(&i.then, doc_mod_id, out);
            if let Some(els) = &i.els {
                walk_hir_block_stmt(els, doc_mod_id, out);
            }
        }
        S::Match(m) => {
            walk_hir_expr(&m.scrutinee, doc_mod_id, out);
            for arm in &m.arms {
                walk_hir_block_stmt(&arm.body, doc_mod_id, out);
            }
        }
        S::While(w) => {
            walk_hir_expr(&w.cond, doc_mod_id, out);
            walk_hir_block_stmt(&w.stmts, doc_mod_id, out);
        }
        S::VarDecl(v) => walk_hir_expr(&v.init, doc_mod_id, out),
        S::Assign(a) => {
            walk_hir_primary(&a.dst, doc_mod_id, out);
            walk_hir_expr(&a.src, doc_mod_id, out);
        }
        S::NovelSyscall(n) => walk_hir_expr(&n.call, doc_mod_id, out),
    }
}

fn walk_hir_block_stmt(b: &biwac_hir::BlockStmt, doc_mod_id: ModId, out: &mut Vec<Classification>) {
    for s in &b.stmts {
        walk_hir_stmt(s, doc_mod_id, out);
    }
}

fn walk_hir_block_expr(b: &biwac_hir::BlockExpr, doc_mod_id: ModId, out: &mut Vec<Classification>) {
    for s in &b.stmts {
        walk_hir_stmt(s, doc_mod_id, out);
    }
    walk_hir_expr(&b.expr, doc_mod_id, out);
}

fn walk_hir_expr(e: &biwac_hir::Expr, doc_mod_id: ModId, out: &mut Vec<Classification>) {
    use biwac_hir::ExprVal as E;
    match &e.expr {
        E::Primary(p) => walk_hir_primary(p, doc_mod_id, out),
        E::Unary(u) => walk_hir_expr(&u.right, doc_mod_id, out),
        E::Binary(b) => {
            walk_hir_expr(&b.left, doc_mod_id, out);
            walk_hir_expr(&b.right, doc_mod_id, out);
        }
    }
}

fn walk_hir_primary(p: &biwac_hir::Primary, doc_mod_id: ModId, out: &mut Vec<Classification>) {
    use biwac_hir::Primary as P;
    match p {
        P::Literal(biwac_hir::Literal::Struct(s)) => {
            for (_, e) in &s.members {
                walk_hir_expr(e, doc_mod_id, out);
            }
        }
        P::Literal(_) => {}
        P::Variable(_) => {}
        P::FnCall(f) => {
            for a in &f.args {
                walk_hir_expr(a, doc_mod_id, out);
            }
        }
        P::MemberAccess(m) => walk_hir_expr(&m.left, doc_mod_id, out),
        P::MethodCall(m) => {
            walk_hir_expr(&m.left, doc_mod_id, out);
            for a in &m.args {
                walk_hir_expr(a, doc_mod_id, out);
            }
            if m.target.get().is_some() && m.method.span.module() == doc_mod_id {
                push(out, m.method.span.clone(), ResolvedKind::Method);
            }
        }
        P::IfExpr(i) => {
            walk_hir_expr(&i.cond, doc_mod_id, out);
            walk_hir_block_expr(&i.then, doc_mod_id, out);
            walk_hir_block_expr(&i.els, doc_mod_id, out);
        }
        P::Match(m) => {
            walk_hir_expr(&m.scrutinee, doc_mod_id, out);
            for arm in &m.arms {
                walk_hir_block_expr(&arm.body, doc_mod_id, out);
            }
        }
        P::Block(b) => walk_hir_block_expr(b, doc_mod_id, out),
        P::VariantCtor(v) => match &v.fields {
            biwac_hir::VariantCtorFields::Unit => {}
            biwac_hir::VariantCtorFields::Positional(exprs) => {
                for e in exprs {
                    walk_hir_expr(e, doc_mod_id, out);
                }
            }
            biwac_hir::VariantCtorFields::Named(fields) => {
                for (_, e) in fields {
                    walk_hir_expr(e, doc_mod_id, out);
                }
            }
        },
    }
}
