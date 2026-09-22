use std::cell::OnceCell;

use biwa_lsp_lexer::SyntaxKind;
use biwac_ast::{
    ArgDecl, ArgDeclList, Attrs, EnumDef, FnDef, Globals, ImplBlock, ImportDecl, MethodArgDeclList,
    MethodDef, ModAst, NovelScene, StructDef, TraitDef, TraitItemArgs, TraitItemDecl, TypeAlias,
    TypeDef, VariantDecl, VariantFieldsDecl,
    symbols::globals::{GenArgDeclItem, GenArgsDecl},
};
use biwac_base::{IdentInterner, ModId, ModPath};
use biwac_span::{GenDefId, LocalGenDefId, Span};

use crate::cursor::{Children, SyntaxNode, elem_kind, intern_ident_token, node_span, token_span};
use crate::error::LowerError;
use crate::novel::lower_novel_stmts;
use crate::path_ty::{lower_ident_path, lower_optional_return_type, lower_type_repr};
use crate::stmt::lower_fn_body;

/// `[T, U: A && B]` (宣言。使用時の型引数指定 `GenericsArgList` とは別物)。
fn lower_generics_arg_decl<I>(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> GenArgsDecl<I> {
    let span = node_span(mod_id, node);
    let mut children = Children::of(node);
    children.eat_token(SyntaxKind::LBracket);

    let mut genargs = Vec::new();
    while let Some(item_node) = children.eat_node(SyntaxKind::GenericsArgItem) {
        let mut item_children = Children::of(&item_node);
        if let Some(id_tok) = item_children.eat_token(SyntaxKind::Ident) {
            let id = intern_ident_token(mod_id, interner, &id_tok);
            let mut bounds = Vec::new();
            if item_children.eat_token(SyntaxKind::Colon).is_some() {
                loop {
                    let Some(ty_node) = item_children.next_node() else {
                        break;
                    };
                    if let Some(t) = lower_type_repr(mod_id, interner, &ty_node, errors) {
                        bounds.push(t);
                    }
                    if item_children.eat_token(SyntaxKind::AmpAmp).is_none() {
                        break;
                    }
                }
            }
            genargs.push(GenArgDeclItem::<I> {
                id,
                def_id: OnceCell::new(),
                bounds,
            });
        }
        if children.eat_token(SyntaxKind::Comma).is_none() {
            break;
        }
    }
    children.eat_token(SyntaxKind::RBracket);

    GenArgsDecl { genargs, span }
}

fn lower_plain_arg_decl_list(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> ArgDeclList {
    let span = node_span(mod_id, node);
    let mut children = Children::of(node);
    children.eat_token(SyntaxKind::LParen);

    let mut args = Vec::new();
    while let Some(id_tok) = children.eat_token(SyntaxKind::Ident) {
        let id = intern_ident_token(mod_id, interner, &id_tok);
        children.eat_token(SyntaxKind::Colon);
        if let Some(ty_node) = children.eat_node(SyntaxKind::TypeRepr) {
            if let Some(typ) = lower_type_repr(mod_id, interner, &ty_node, errors) {
                let arg_span = Span::merge(&id.span, &typ.span);
                args.push(ArgDecl {
                    typ,
                    id,
                    span: arg_span,
                    var_id: OnceCell::new(),
                });
            }
        }
        if children.eat_token(SyntaxKind::Comma).is_none() {
            break;
        }
    }
    children.eat_token(SyntaxKind::RParen);

    ArgDeclList { args, span }
}

fn lower_method_arg_decl_list(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<MethodArgDeclList> {
    let span = node_span(mod_id, node);
    let mut children = Children::of(node);
    children.eat_token(SyntaxKind::LParen);
    let self_tok = children.eat_token(SyntaxKind::KwSelf)?;
    let self_span = token_span(mod_id, &self_tok);
    children.eat_token(SyntaxKind::Comma);

    let mut args = Vec::new();
    while let Some(id_tok) = children.eat_token(SyntaxKind::Ident) {
        let id = intern_ident_token(mod_id, interner, &id_tok);
        children.eat_token(SyntaxKind::Colon);
        if let Some(ty_node) = children.eat_node(SyntaxKind::TypeRepr) {
            if let Some(typ) = lower_type_repr(mod_id, interner, &ty_node, errors) {
                let arg_span = Span::merge(&id.span, &typ.span);
                args.push(ArgDecl {
                    typ,
                    id,
                    span: arg_span,
                    var_id: OnceCell::new(),
                });
            }
        }
        if children.eat_token(SyntaxKind::Comma).is_none() {
            break;
        }
    }
    children.eat_token(SyntaxKind::RParen);

    Some(MethodArgDeclList {
        self_span,
        args,
        span,
    })
}

fn lower_fn_def(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<FnDef> {
    let span = node_span(mod_id, node);
    let mut children = Children::of(node);
    children.eat_token(SyntaxKind::KwFn);
    let id_tok = children.eat_token(SyntaxKind::Ident)?;
    let id = intern_ident_token(mod_id, interner, &id_tok);

    let genargs = children
        .eat_node(SyntaxKind::GenericsArgDecl)
        .map(|n| lower_generics_arg_decl::<LocalGenDefId>(mod_id, interner, &n, errors));

    let args_node = children.eat_node(SyntaxKind::FunctionArgDecl)?;
    let args = lower_plain_arg_decl_list(mod_id, interner, &args_node, errors);
    let rtype = lower_optional_return_type(mod_id, interner, &mut children, &args.span, errors)?;

    let body_node = children.eat_node(SyntaxKind::BlockStmt)?;
    let (stmts, expr, _) = lower_fn_body(mod_id, interner, &body_node, errors);

    Some(FnDef {
        id,
        def_id: OnceCell::new(),
        args,
        stmts,
        expr,
        rtype,
        span,
        attrs: Attrs::empty(),
        genargs,
    })
}

fn lower_method_def(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<MethodDef> {
    let span = node_span(mod_id, node);
    let mut children = Children::of(node);
    children.eat_token(SyntaxKind::KwFn);
    let id_tok = children.eat_token(SyntaxKind::Ident)?;
    let id = intern_ident_token(mod_id, interner, &id_tok);

    let genargs = children
        .eat_node(SyntaxKind::GenericsArgDecl)
        .map(|n| lower_generics_arg_decl::<LocalGenDefId>(mod_id, interner, &n, errors));

    let args_node = children.eat_node(SyntaxKind::MethodArgDecl)?;
    let args = lower_method_arg_decl_list(mod_id, interner, &args_node, errors)?;
    let rtype = lower_optional_return_type(mod_id, interner, &mut children, &args.span, errors)?;

    let body_node = children.eat_node(SyntaxKind::BlockStmt)?;
    let (stmts, expr, _) = lower_fn_body(mod_id, interner, &body_node, errors);

    Some(MethodDef {
        def_id: OnceCell::new(),
        id,
        args,
        stmts,
        expr,
        rtype,
        span,
        attrs: Attrs::empty(),
        genargs,
    })
}

fn lower_struct_def(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<StructDef> {
    let mut children = Children::of(node);
    children.eat_token(SyntaxKind::KwStruct);
    let id_tok = children.eat_token(SyntaxKind::Ident)?;
    let id = intern_ident_token(mod_id, interner, &id_tok);

    let genargs = children
        .eat_node(SyntaxKind::GenericsArgDecl)
        .map(|n| lower_generics_arg_decl::<GenDefId>(mod_id, interner, &n, errors));

    children.eat_token(SyntaxKind::LBrace);
    let mut members = Vec::new();
    while let Some(member_tok) = children.eat_token(SyntaxKind::Ident) {
        children.eat_token(SyntaxKind::Colon);
        if let Some(ty_node) = children.eat_node(SyntaxKind::TypeRepr) {
            if let Some(typ) = lower_type_repr(mod_id, interner, &ty_node, errors) {
                let member_id = intern_ident_token(mod_id, interner, &member_tok);
                members.push((member_id, typ));
            }
        }
        if children.eat_token(SyntaxKind::Comma).is_none() {
            break;
        }
    }
    children.eat_token(SyntaxKind::RBrace);

    Some(StructDef {
        id,
        def_id: OnceCell::new(),
        members,
        genargs,
        attrs: Attrs::empty(),
    })
}

/// `type X = <type>;` / `type X[T] = <type>;`
fn lower_type_alias_def(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<TypeAlias> {
    let mut children = Children::of(node);
    children.eat_token(SyntaxKind::KwType);
    let id_tok = children.eat_token(SyntaxKind::Ident)?;
    let ident = intern_ident_token(mod_id, interner, &id_tok);

    let genargs = children
        .eat_node(SyntaxKind::GenericsArgDecl)
        .map(|n| lower_generics_arg_decl::<GenDefId>(mod_id, interner, &n, errors));

    children.eat_token(SyntaxKind::Eq);
    let right_node = children.eat_node(SyntaxKind::TypeRepr)?;
    let right = lower_type_repr(mod_id, interner, &right_node, errors)?;
    children.eat_token(SyntaxKind::Semi);

    Some(TypeAlias {
        ident,
        def_id: OnceCell::new(),
        genargs,
        right,
        attrs: Attrs::empty(),
    })
}

// ── enum ─────────────────────────────────────────────────────────────────────
// `docs/enum-and-match.md`

fn lower_enum_def(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<EnumDef> {
    let mut children = Children::of(node);
    children.eat_token(SyntaxKind::KwEnum);
    let id_tok = children.eat_token(SyntaxKind::Ident)?;
    let id = intern_ident_token(mod_id, interner, &id_tok);

    let genargs = children
        .eat_node(SyntaxKind::GenericsArgDecl)
        .map(|n| lower_generics_arg_decl::<GenDefId>(mod_id, interner, &n, errors));

    children.eat_token(SyntaxKind::LBrace);
    let mut variants = Vec::new();
    while let Some(v_node) = children.eat_node(SyntaxKind::VariantDecl) {
        if let Some(v) = lower_variant_decl(mod_id, interner, &v_node, errors) {
            variants.push(v);
        }
        if children.eat_token(SyntaxKind::Comma).is_none() {
            break;
        }
    }
    children.eat_token(SyntaxKind::RBrace);

    Some(EnumDef {
        id,
        def_id: OnceCell::new(),
        variants,
        genargs,
        attrs: Attrs::empty(),
    })
}

/// `Red` (unit) / `Rgb(Int, Int, Int)` (tuple) / `Named { name: String }` (struct)
fn lower_variant_decl(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<VariantDecl> {
    let span = node_span(mod_id, node);
    let mut children = Children::of(node);
    let id_tok = children.eat_token(SyntaxKind::Ident)?;
    let id = intern_ident_token(mod_id, interner, &id_tok);

    let fields = match children.peek_kind() {
        Some(SyntaxKind::LParen) => {
            children.eat_token(SyntaxKind::LParen);
            let mut typs = Vec::new();
            while let Some(ty_node) = children.eat_node(SyntaxKind::TypeRepr) {
                if let Some(t) = lower_type_repr(mod_id, interner, &ty_node, errors) {
                    typs.push(t);
                }
                if children.eat_token(SyntaxKind::Comma).is_none() {
                    break;
                }
            }
            children.eat_token(SyntaxKind::RParen);

            // タプル形式のフィールド名は `_0`, `_1` … 正規化する。
            // biwac_parser もパーサ (interner を持つ側) で同じことをしている
            // (`docs/enum-and-match.md`: 「タプル形式のフィールド名はパーサが付ける」)。
            let named = typs
                .into_iter()
                .enumerate()
                .map(|(i, ty)| {
                    let name = interner.get_or_insert(&format!("_{i}"));
                    let ty_span = ty.span.clone();
                    (
                        biwac_ast::Ident {
                            id: name,
                            span: ty_span,
                        },
                        ty,
                    )
                })
                .collect();
            VariantFieldsDecl::Tuple(named)
        }
        Some(SyntaxKind::LBrace) => {
            children.eat_token(SyntaxKind::LBrace);
            let mut members = Vec::new();
            while let Some(member_tok) = children.eat_token(SyntaxKind::Ident) {
                children.eat_token(SyntaxKind::Colon);
                if let Some(ty_node) = children.eat_node(SyntaxKind::TypeRepr) {
                    if let Some(t) = lower_type_repr(mod_id, interner, &ty_node, errors) {
                        members.push((intern_ident_token(mod_id, interner, &member_tok), t));
                    }
                }
                if children.eat_token(SyntaxKind::Comma).is_none() {
                    break;
                }
            }
            children.eat_token(SyntaxKind::RBrace);
            VariantFieldsDecl::Struct(members)
        }
        _ => VariantFieldsDecl::Unit,
    };

    Some(VariantDecl {
        id,
        def_id: OnceCell::new(),
        fields,
        span,
    })
}

// ── trait ────────────────────────────────────────────────────────────────────
// `docs/trait.md` (第1段)

fn lower_trait_def(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<TraitDef> {
    let span = node_span(mod_id, node);
    let mut children = Children::of(node);
    children.eat_token(SyntaxKind::KwTrait);
    let id_tok = children.eat_token(SyntaxKind::Ident)?;
    let id = intern_ident_token(mod_id, interner, &id_tok);

    let genargs = children
        .eat_node(SyntaxKind::GenericsArgDecl)
        .map(|n| lower_generics_arg_decl::<GenDefId>(mod_id, interner, &n, errors));

    children.eat_token(SyntaxKind::LBrace);
    let mut items = Vec::new();
    loop {
        match children.peek_kind() {
            None | Some(SyntaxKind::RBrace) => break,
            Some(SyntaxKind::TraitItemDecl) => {
                let n = children.eat_node(SyntaxKind::TraitItemDecl)?;
                if let Some(item) = lower_trait_item_decl(mod_id, interner, &n, errors) {
                    items.push(item);
                }
            }
            _ => {
                children.next_elem();
            }
        }
    }
    children.eat_token(SyntaxKind::RBrace);

    Some(TraitDef {
        id,
        def_id: OnceCell::new(),
        self_gen: OnceCell::new(),
        items,
        genargs,
        attrs: Attrs::empty(),
        span,
    })
}

/// trait の項目。`self` を取ればメソッド形式 (`MethodArgDecl`)、
/// 取らなければ関連関数形式 (`FunctionArgDecl`) — どちらの CST ノードが
/// 現れたかで判定できる (biwa_lsp_parser 側が `is_method_def` で先読み済み)。
fn lower_trait_item_decl(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<TraitItemDecl> {
    let span = node_span(mod_id, node);
    let mut children = Children::of(node);
    children.eat_token(SyntaxKind::KwFn);
    let id_tok = children.eat_token(SyntaxKind::Ident)?;
    let id = intern_ident_token(mod_id, interner, &id_tok);

    let genargs = children
        .eat_node(SyntaxKind::GenericsArgDecl)
        .map(|n| lower_generics_arg_decl::<LocalGenDefId>(mod_id, interner, &n, errors));

    let args = if let Some(n) = children.eat_node(SyntaxKind::MethodArgDecl) {
        TraitItemArgs::Method(lower_method_arg_decl_list(mod_id, interner, &n, errors)?)
    } else if let Some(n) = children.eat_node(SyntaxKind::FunctionArgDecl) {
        TraitItemArgs::Assoc(lower_plain_arg_decl_list(mod_id, interner, &n, errors))
    } else {
        errors.push(LowerError::new(
            "expected an argument list in this trait item",
            span,
        ));
        return None;
    };
    let args_span = match &args {
        TraitItemArgs::Assoc(a) => a.span.clone(),
        TraitItemArgs::Method(a) => a.span.clone(),
    };

    let rtype = lower_optional_return_type(mod_id, interner, &mut children, &args_span, errors)?;
    children.eat_token(SyntaxKind::Semi);

    Some(TraitItemDecl {
        id,
        def_id: OnceCell::new(),
        args,
        rtype,
        genargs,
        attrs: Attrs::empty(),
        span,
    })
}

fn lower_import_decl(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<Globals> {
    let span = node_span(mod_id, node);
    let mut children = Children::of(node);
    children.eat_token(SyntaxKind::KwImport);
    let path_node = children.eat_node(SyntaxKind::IdentPath)?;
    let path = lower_ident_path(mod_id, interner, &path_node)?;

    if children.eat_token(SyntaxKind::KwAs).is_some() {
        children.eat_token(SyntaxKind::Ident);
        errors.push(LowerError::new(
            "import aliases (`as`) have no representation in the compiler AST yet; \
             the alias was dropped",
            span.clone(),
        ));
    }
    children.eat_token(SyntaxKind::Semi);

    Some(Globals::Import(ImportDecl { path, span }))
}

fn lower_impl_block(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<Globals> {
    let span = node_span(mod_id, node);
    let mut children = Children::of(node);
    children.eat_token(SyntaxKind::KwImpl);

    let genargs_decl = children
        .eat_node(SyntaxKind::GenericsArgDecl)
        .map(|n| lower_generics_arg_decl::<LocalGenDefId>(mod_id, interner, &n, errors));

    let self_ty_node = children.eat_node(SyntaxKind::TypeRepr)?;
    let self_typ = lower_type_repr(mod_id, interner, &self_ty_node, errors)?;

    // `impl Ty: Trait { .. }` (`docs/trait.md`)。
    let trait_typ = if children.eat_token(SyntaxKind::Colon).is_some() {
        children
            .eat_node(SyntaxKind::TypeRepr)
            .and_then(|n| lower_type_repr(mod_id, interner, &n, errors))
    } else {
        None
    };

    children.eat_token(SyntaxKind::LBrace);

    let mut assoc_fns = Vec::new();
    let mut methods = Vec::new();

    loop {
        match children.peek_kind() {
            None | Some(SyntaxKind::RBrace) => break,
            Some(SyntaxKind::FunctionDef) => {
                let n = children.eat_node(SyntaxKind::FunctionDef)?;
                if let Some(f) = lower_fn_def(mod_id, interner, &n, errors) {
                    assoc_fns.push(f);
                }
            }
            Some(SyntaxKind::MethodDef) => {
                let n = children.eat_node(SyntaxKind::MethodDef)?;
                if let Some(m) = lower_method_def(mod_id, interner, &n, errors) {
                    methods.push(m);
                }
            }
            _ => {
                // `Error` ノードなどエラー回復の残骸。読み飛ばす。
                children.next_elem();
            }
        }
    }
    children.eat_token(SyntaxKind::RBrace);

    Some(Globals::ImplBlock(ImplBlock {
        impl_id: OnceCell::new(),
        assoc_fns,
        methods,
        // `{{ .. }}` によるネイティブ実装は biwa-lsp-parser がまだ読まない。
        native_assoc_fns: vec![],
        native_methods: vec![],
        genargs_decl,
        self_typ,
        trait_typ,
        span,
    }))
}

fn lower_scene_def(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<Globals> {
    let span = node_span(mod_id, node);
    let mut children = Children::of(node);
    children.eat_token(SyntaxKind::KwScene);
    let id_tok = children.eat_token(SyntaxKind::Ident)?;
    let id = intern_ident_token(mod_id, interner, &id_tok);

    let args_node = children.eat_node(SyntaxKind::FunctionArgDecl)?;
    let args = lower_plain_arg_decl_list(mod_id, interner, &args_node, errors);
    let rtype = lower_optional_return_type(mod_id, interner, &mut children, &args.span, errors)?;

    children.eat_token(SyntaxKind::DoubleLBrace);
    let body_node = children.eat_node(SyntaxKind::NovelModeBody);
    let stmts = match &body_node {
        Some(n) => lower_novel_stmts(mod_id, interner, n, errors),
        None => vec![],
    };
    children.eat_token(SyntaxKind::DoubleRBrace);

    Some(Globals::NovelScene(NovelScene {
        id,
        def_id: OnceCell::new(),
        args,
        rtype,
        stmts,
        span,
        attrs: Attrs::empty(),
    }))
}

/// `Root` ノードを 1 モジュール分の `biwac_ast::ModAst` に直す。
///
/// 対応できない構文要素は `errors` に積んで読み飛ばす。1 箇所が
/// 表現できないからといって、そのファイル全体の lowering を諦めることはしない。
pub fn lower_module(
    mod_id: ModId,
    modpath: ModPath,
    interner: &mut IdentInterner,
    root: &SyntaxNode,
) -> (ModAst, Vec<LowerError>) {
    let mut errors = Vec::new();
    let mut globals = Vec::new();
    let mut children = Children::of(root);

    while let Some(elem) = children.peek() {
        match elem_kind(&elem) {
            SyntaxKind::ImportDecl => {
                let n = children.eat_node(SyntaxKind::ImportDecl).expect("peeked");
                if let Some(g) = lower_import_decl(mod_id, interner, &n, &mut errors) {
                    globals.push(g);
                }
            }
            SyntaxKind::FunctionDef => {
                let n = children.eat_node(SyntaxKind::FunctionDef).expect("peeked");
                if let Some(f) = lower_fn_def(mod_id, interner, &n, &mut errors) {
                    globals.push(Globals::FnDef(f));
                }
            }
            SyntaxKind::StructDef => {
                let n = children.eat_node(SyntaxKind::StructDef).expect("peeked");
                if let Some(s) = lower_struct_def(mod_id, interner, &n, &mut errors) {
                    globals.push(Globals::TypeDef(TypeDef::Struct(s)));
                }
            }
            SyntaxKind::EnumDef => {
                let n = children.eat_node(SyntaxKind::EnumDef).expect("peeked");
                if let Some(e) = lower_enum_def(mod_id, interner, &n, &mut errors) {
                    globals.push(Globals::TypeDef(TypeDef::Enum(e)));
                }
            }
            SyntaxKind::TraitDef => {
                let n = children.eat_node(SyntaxKind::TraitDef).expect("peeked");
                if let Some(t) = lower_trait_def(mod_id, interner, &n, &mut errors) {
                    globals.push(Globals::TraitDef(t));
                }
            }
            SyntaxKind::TypeAliasDef => {
                let n = children.eat_node(SyntaxKind::TypeAliasDef).expect("peeked");
                if let Some(t) = lower_type_alias_def(mod_id, interner, &n, &mut errors) {
                    globals.push(Globals::TypeDef(TypeDef::TypeAlias(t)));
                }
            }
            SyntaxKind::ImplBlock => {
                let n = children.eat_node(SyntaxKind::ImplBlock).expect("peeked");
                if let Some(g) = lower_impl_block(mod_id, interner, &n, &mut errors) {
                    globals.push(g);
                }
            }
            SyntaxKind::SceneDef => {
                let n = children.eat_node(SyntaxKind::SceneDef).expect("peeked");
                if let Some(g) = lower_scene_def(mod_id, interner, &n, &mut errors) {
                    globals.push(g);
                }
            }
            _ => {
                // トップレベルの `Error` ノードなど。読み飛ばす。
                children.next_elem();
            }
        }
    }

    (ModAst { modpath, globals }, errors)
}
