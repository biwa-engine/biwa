use std::cell::OnceCell;

use biwa_lsp_lexer::SyntaxKind;
use biwac_ast::{
    ArgDecl, ArgDeclList, Attrs, FnDef, Globals, ImplBlock, ImportDecl, MethodArgDeclList,
    MethodDef, ModAst, NovelScene, StructDef, TypeDef,
    symbols::globals::{GenArgDeclItem, GenArgsDecl},
};
use biwac_base::{IdentInterner, ModId, ModPath};
use biwac_span::{LocalGenDefId, Span};

use crate::cursor::{Children, SyntaxNode, elem_kind, intern_ident_token, node_span, token_span};
use crate::error::LowerError;
use crate::path_ty::{lower_ident_path, lower_ret_type_repr, lower_type_repr};
use crate::stmt::lower_fn_body;

fn lower_generics_arg_decl<I>(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
) -> GenArgsDecl<I> {
    let span = node_span(mod_id, node);
    let mut children = Children::of(node);
    children.eat_token(SyntaxKind::LBracket);

    let mut genargs = Vec::new();
    while let Some(id_tok) = children.eat_token(SyntaxKind::Ident) {
        let id = intern_ident_token(mod_id, interner, &id_tok);
        genargs.push(GenArgDeclItem::<I> {
            id,
            def_id: OnceCell::new(),
            // `T: A && B` の制限は biwa-lsp-parser の `GenericsArgDecl` がまだ
            // 読まないので常に空になる。
            bounds: vec![],
        });
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
        .map(|n| lower_generics_arg_decl::<LocalGenDefId>(mod_id, interner, &n));

    let args_node = children.eat_node(SyntaxKind::FunctionArgDecl)?;
    let args = lower_plain_arg_decl_list(mod_id, interner, &args_node, errors);

    children.eat_token(SyntaxKind::Arrow);
    let ret_node = children.eat_node(SyntaxKind::TypeRepr)?;
    let rtype = lower_ret_type_repr(mod_id, interner, &ret_node, errors)?;

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
        .map(|n| lower_generics_arg_decl::<LocalGenDefId>(mod_id, interner, &n));

    let args_node = children.eat_node(SyntaxKind::MethodArgDecl)?;
    let args = lower_method_arg_decl_list(mod_id, interner, &args_node, errors)?;

    children.eat_token(SyntaxKind::Arrow);
    let ret_node = children.eat_node(SyntaxKind::TypeRepr)?;
    let rtype = lower_ret_type_repr(mod_id, interner, &ret_node, errors)?;

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
        .map(|n| lower_generics_arg_decl(mod_id, interner, &n));

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
        .map(|n| lower_generics_arg_decl::<LocalGenDefId>(mod_id, interner, &n));

    let self_ty_node = children.eat_node(SyntaxKind::TypeRepr)?;
    let self_typ = lower_type_repr(mod_id, interner, &self_ty_node, errors)?;

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
        // `impl Foo: Trait { .. }` の `: Trait` は biwa-lsp-parser がまだ読まない。
        trait_typ: None,
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

    children.eat_token(SyntaxKind::Arrow);
    let ret_node = children.eat_node(SyntaxKind::TypeRepr)?;
    let rtype = lower_ret_type_repr(mod_id, interner, &ret_node, errors)?;

    children.eat_token(SyntaxKind::DoubleLBrace);
    // ノベルモードの本体 (`NovelModeBody`) は今のところ構造を持たないトークンの
    // 塊でしかなく (biwac_novel_parser 相当の文法が biwa-lsp-parser に無い)、
    // `biwac_ast::NovelStmt` へは変換できない。空の本体として salvage する。
    children.eat_node(SyntaxKind::NovelModeBody);
    children.eat_token(SyntaxKind::DoubleRBrace);

    errors.push(LowerError::new(
        "the novel-mode scene body was not lowered: biwa-lsp-parser does not yet parse \
         novel-mode structure the way `biwac_novel_parser` does, so this scene has an \
         empty body",
        span.clone(),
    ));

    Some(Globals::NovelScene(NovelScene {
        id,
        def_id: OnceCell::new(),
        args,
        rtype,
        stmts: vec![],
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
            SyntaxKind::TypeAliasDef => {
                let n = children.eat_node(SyntaxKind::TypeAliasDef).expect("peeked");
                errors.push(LowerError::new(
                    "type alias right-hand side (`= <type>`) is not parsed by \
                     biwa-lsp-parser yet; this declaration was dropped",
                    node_span(mod_id, &n),
                ));
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
