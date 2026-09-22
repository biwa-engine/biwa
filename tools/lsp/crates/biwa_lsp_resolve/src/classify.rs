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
//! - メソッド呼び出し (`.foo()`) は分類しない (要求仕様どおり: 型推論をしないと
//!   実装が決まらないことが多いため、この段では variable 系のまま残す)。
//!   `MethodCall::target` に相当する解決は名前解決の範囲外である。
//! - `scene` の本体は `biwa_lsp_lower` がまだ構造化していないため
//!   (`biwa_lsp_lower::lib` の既知の非対応リスト参照)、シグニチャ以外は
//!   分類しようがない。
//! - `Self` 型 (`TypReprVal::SelfTyp`) は `OnceCell` を持たないため分類しない。

use std::collections::HashSet;

use biwac_ast::symbols::globals::GenArgsDecl;
use biwac_ast::{
    AbsolutePathHeader, ArgDeclList, BlockExpr, BlockStmt, Exprs, FnDef, Globals, ImplBlock,
    Literal, MethodArgDeclList, MethodDef, NativeFnDef, NativeMethodDef, NovelScene, Path,
    PathSegmentResolution, Pattern, PatternFields, Primary, RetTypRepr, Stmt, TraitDef,
    TraitItemArgs, TraitItemDecl, TypDecl, TypRepr, TypReprVal, TypeDef, Variable, VariantDecl,
    VariantFieldsDecl,
};
use biwac_base::ModId;
use biwac_package_loader::{LoadedModule, Pkg};
use biwac_span::{DefIdKind, Span, VarId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedKind {
    Function,
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
    // s.stmts (NovelStmt) は biwa_lsp_lower がまだ本体を構造化しないので常に空
    // (`biwa_lsp_lower::lib` の既知の非対応リスト参照)。
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
