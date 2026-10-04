use biwa_lsp_parser::parse;
use biwac_ast::{Exprs, Globals, Primary, Stmt, TypeDef};
use biwac_base::{IdentInterner, ModId, ModPath};

fn lower(src: &str) -> (biwac_ast::ModAst, Vec<biwa_lsp_lower::LowerError>) {
    let parse_result = parse(src);
    let root = parse_result.syntax();
    let mut interner = IdentInterner::new();
    biwa_lsp_lower::lower_module(ModId::new_in_self(0), ModPath::Main, &mut interner, &root)
}

#[test]
fn lowers_function_with_binary_expr_tail() {
    let (ast, errors) = lower("fn add(x: Int, y: Int) -> Int { x + y }");
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    assert_eq!(ast.globals.len(), 1);
    let Globals::FnDef(f) = &ast.globals[0] else {
        panic!("expected FnDef, got {:?}", ast.globals[0]);
    };
    assert_eq!(f.args.args.len(), 2);
    assert!(f.stmts.is_empty());
    assert!(matches!(f.expr, Some(Exprs::Binary(_))));
}

#[test]
fn lowers_struct_def() {
    let (ast, errors) = lower("struct Point { x: Int, y: Int, }");
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    assert_eq!(ast.globals.len(), 1);
    let Globals::TypeDef(TypeDef::Struct(s)) = &ast.globals[0] else {
        panic!("expected struct def, got {:?}", ast.globals[0]);
    };
    assert_eq!(s.members.len(), 2);
}

#[test]
fn lowers_import_decl() {
    let (ast, errors) = lower("import foo::bar::Baz;");
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    assert_eq!(ast.globals.len(), 1);
    assert!(matches!(ast.globals[0], Globals::Import(_)));
}

#[test]
fn import_alias_is_dropped_with_an_error() {
    let (ast, errors) = lower("import foo::Bar as B;");
    // パスの取り込み自体は成功するが、エイリアスは biwac_ast に表現が無いので
    // エラーとして記録されたうえで捨てられる。
    assert_eq!(ast.globals.len(), 1);
    assert!(matches!(ast.globals[0], Globals::Import(_)));
    assert!(!errors.is_empty());
}

#[test]
fn lowers_impl_block_with_assoc_fn_and_method() {
    let src = r#"
impl Point {
  fn new(x: Int, y: Int) -> Point {
    Point { x = x, y = y }
  }
  fn scale(self, n: Int) -> Int {
    self.x * n
  }
}
"#;
    let (ast, errors) = lower(src);
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    assert_eq!(ast.globals.len(), 1);
    let Globals::ImplBlock(imp) = &ast.globals[0] else {
        panic!("expected impl block, got {:?}", ast.globals[0]);
    };
    assert_eq!(imp.assoc_fns.len(), 1);
    assert_eq!(imp.methods.len(), 1);

    // `Point { x = x, y = y }` は struct literal として tail に来る。
    assert!(matches!(
        imp.assoc_fns[0].expr,
        Some(Exprs::Primary(Primary::Literal(_)))
    ));

    // `self.x * n` は `self.x` (MemberAccess) を左辺に持つ二項式になる。
    let Some(Exprs::Binary(bin)) = &imp.methods[0].expr else {
        panic!("expected binary expr tail in `scale`");
    };
    let Exprs::Primary(Primary::MemberAccess(member_access)) = &*bin.left else {
        panic!(
            "expected `self.x` member access on the left of `*`, got {:?}",
            bin.left
        );
    };
    assert!(matches!(
        *member_access.left,
        Exprs::Primary(Primary::Variable(biwac_ast::Variable::SelfVar(_)))
    ));
}

#[test]
fn lowers_if_else_as_statement_and_expression() {
    let (ast, errors) = lower(
        r#"
fn f(x: Int) -> Int {
  let y = if x > 0 { x } else { 0 - x };
  if x > 0 {
    y = y;
  } else {
    y = y;
  }
  y
}
"#,
    );
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    let Globals::FnDef(f) = &ast.globals[0] else {
        panic!("expected fn def");
    };
    assert_eq!(f.stmts.len(), 2);
    assert!(matches!(f.stmts[0], Stmt::VarDecl(_)));
    assert!(matches!(f.stmts[1], Stmt::If(_)));
    assert!(matches!(f.expr, Some(Exprs::Primary(Primary::Variable(_)))));
}

#[test]
fn desugars_else_if_chain_in_expression_position() {
    // 注意: biwa-lsp-parser の `parse_statement_or_expr` はブロック内の `if` を
    // 常に `IfStmt` (文形) としてパースし、ブロックの「最後の要素だから式として
    // 扱う」という判定を持たない。そのため `fn f() -> Int { if .. {..} else {..} }`
    // のように if を素のまま tail に置く書き方は、今の CST では式としての
    // `IfExpr` にならない (これも biwa-lsp-parser 側の既知のギャップ)。
    // `let` の右辺に置けば `parse_expression` 経由で `IfExpr` になるので、
    // ここではその形で else-if の畳み込みを確認する。
    let (ast, errors) = lower(
        r#"
fn sign(x: Int) -> Int {
  let r = if x > 0 { 1 } else if x < 0 { 0 - 1 } else { 0 };
  r
}
"#,
    );
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    let Globals::FnDef(f) = &ast.globals[0] else {
        panic!("expected fn def");
    };
    assert_eq!(f.stmts.len(), 1);
    let Stmt::VarDecl(v) = &f.stmts[0] else {
        panic!("expected a var decl, got {:?}", f.stmts[0]);
    };
    let Exprs::Primary(Primary::IfExpr(outer)) = &v.init else {
        panic!("expected an if-expression initializer, got {:?}", v.init);
    };
    // `else if` は `else { if .. }` として畳み込まれているはず。
    assert!(matches!(
        *outer.els.expr,
        Exprs::Primary(Primary::IfExpr(_))
    ));
}

#[test]
fn unsupported_constructs_are_salvaged_with_errors_not_a_hard_failure() {
    // 論理演算子 `&&` は biwac_ast::BinOperator に対応するバリアントが無い。
    // その式だけ lowering に失敗して tail が None になるが、
    // モジュール全体の lowering は止まらない (後続の Globals も読める)。
    let (ast, errors) = lower(
        r#"
fn broken() -> Bool { TRUE && FALSE }
struct StillParses { a: Int }
"#,
    );
    assert!(!errors.is_empty());
    assert_eq!(ast.globals.len(), 2);
    let Globals::FnDef(f) = &ast.globals[0] else {
        panic!("expected fn def");
    };
    assert!(f.expr.is_none());
    assert!(matches!(
        ast.globals[1],
        Globals::TypeDef(TypeDef::Struct(_))
    ));
}

#[test]
fn lowers_type_alias_def() {
    let (ast, errors) = lower("type MyInt = Int;");
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    assert_eq!(ast.globals.len(), 1);
    let Globals::TypeDef(TypeDef::TypeAlias(alias)) = &ast.globals[0] else {
        panic!("expected a type alias, got {:?}", ast.globals[0]);
    };
    assert!(matches!(
        alias.right.val,
        biwac_ast::TypReprVal::Primitive(biwac_ast::PrimTyp::Int)
    ));
}

#[test]
fn lowers_type_alias_with_generics_and_genarg_type() {
    let (ast, errors) = lower("type Boxed[T] = Box[T];");
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    let Globals::TypeDef(TypeDef::TypeAlias(alias)) = &ast.globals[0] else {
        panic!("expected a type alias");
    };
    assert!(alias.genargs.is_some());
    assert!(matches!(alias.right.val, biwac_ast::TypReprVal::Defined(_)));
}

#[test]
fn lowers_fn_type() {
    let (ast, errors) = lower("struct S { f: fn(Int, Bool) -> Int, g: fn(Int), }");
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    let Globals::TypeDef(TypeDef::Struct(s)) = &ast.globals[0] else {
        panic!("expected struct def, got {:?}", ast.globals[0]);
    };
    let fn_typ = |i: usize| match &s.members[i].1.val {
        biwac_ast::TypReprVal::Fn(f) => f.clone(),
        other => panic!("expected a fn type, got {other:?}"),
    };
    let f = fn_typ(0);
    assert_eq!(f.args.len(), 2);
    assert!(f.rty.is_some());
    // 戻り値を省略した関数型は Void を返す。
    let g = fn_typ(1);
    assert_eq!(g.args.len(), 1);
    assert!(g.rty.is_none());
}

#[test]
fn omitted_arrow_lowers_to_void_return_type() {
    let (ast, errors) = lower("fn nothing() {}");
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    let Globals::FnDef(f) = &ast.globals[0] else {
        panic!("expected a fn def");
    };
    assert!(matches!(f.rtype, biwac_ast::RetTypRepr::Void(_)));
}

#[test]
fn explicit_void_named_type_is_a_defined_type_not_the_void_variant() {
    // 実コンパイラに `Void` というキーワードは無いので、`-> Void` は
    // 「`Void` という名前の型を参照する」普通の戻り値注釈になる
    // (未解決な名前だが、それは名前解決の仕事であって構文エラーではない)。
    let (ast, errors) = lower("fn f() -> Void { 1 }");
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    let Globals::FnDef(f) = &ast.globals[0] else {
        panic!("expected a fn def");
    };
    let biwac_ast::RetTypRepr::Typ(t) = &f.rtype else {
        panic!("expected an explicit return type, got {:?}", f.rtype);
    };
    assert!(matches!(t.val, biwac_ast::TypReprVal::Defined(_)));
}

#[test]
fn lowers_fn_call_and_method_chain() {
    let (ast, errors) = lower(
        r#"
fn f() -> Int {
  make().scale(2).finish()
}
"#,
    );
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    let Globals::FnDef(f) = &ast.globals[0] else {
        panic!("expected fn def");
    };
    let Some(Exprs::Primary(Primary::MethodCall(outer))) = &f.expr else {
        panic!("expected a method call tail, got {:?}", f.expr);
    };
    assert_eq!(outer.args.len(), 0);
    let Exprs::Primary(Primary::MethodCall(inner)) = &*outer.left else {
        panic!("expected a nested method call");
    };
    assert_eq!(inner.args.len(), 1);
    assert!(matches!(*inner.left, Exprs::Primary(Primary::FnCall(_))));
}

#[test]
fn lowers_enum_def() {
    let (ast, errors) = lower(
        r#"
enum Color {
  Red,
  Rgb(Int, Int, Int),
  Named { name: String, alpha: Int },
}
"#,
    );
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    assert_eq!(ast.globals.len(), 1);
    let Globals::TypeDef(TypeDef::Enum(e)) = &ast.globals[0] else {
        panic!("expected an enum def, got {:?}", ast.globals[0]);
    };
    assert_eq!(e.variants.len(), 3);
    assert!(matches!(
        e.variants[0].fields,
        biwac_ast::VariantFieldsDecl::Unit
    ));
    let biwac_ast::VariantFieldsDecl::Tuple(tuple_fields) = &e.variants[1].fields else {
        panic!("expected tuple fields");
    };
    assert_eq!(tuple_fields.len(), 3);
    let biwac_ast::VariantFieldsDecl::Struct(struct_fields) = &e.variants[2].fields else {
        panic!("expected struct fields");
    };
    assert_eq!(struct_fields.len(), 2);
}

#[test]
fn lowers_match_statement_and_expression() {
    let (ast, errors) = lower(
        r#"
fn describe(c: Color) -> Int {
  match c {
    Color::Red => { }
    Color::Rgb(r, g, b) => { }
    Color::Named { name = n, alpha } => { }
    _ => { }
  }
  let n = match c {
    Color::Red => 0,
    Color::Rgb(r, g, b) => r,
    _ => 0,
  };
  n
}
"#,
    );
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    let Globals::FnDef(f) = &ast.globals[0] else {
        panic!("expected fn def");
    };
    let Stmt::Match(match_stmt) = &f.stmts[0] else {
        panic!("expected a match statement, got {:?}", f.stmts[0]);
    };
    assert_eq!(match_stmt.arms.len(), 4);
    assert!(matches!(
        match_stmt.arms[3].pattern,
        biwac_ast::Pattern::Wildcard(_)
    ));
    let biwac_ast::Pattern::Variant(variant_pat) = &match_stmt.arms[1].pattern else {
        panic!("expected a variant pattern");
    };
    assert!(matches!(
        variant_pat.fields,
        biwac_ast::PatternFields::Tuple(ref v) if v.len() == 3
    ));
    let biwac_ast::Pattern::Variant(named_pat) = &match_stmt.arms[2].pattern else {
        panic!("expected a variant pattern");
    };
    let biwac_ast::PatternFields::Struct(struct_fields) = &named_pat.fields else {
        panic!("expected struct fields");
    };
    assert_eq!(struct_fields.len(), 2);

    let Stmt::VarDecl(v) = &f.stmts[1] else {
        panic!("expected a var decl, got {:?}", f.stmts[1]);
    };
    let Exprs::Primary(Primary::Match(match_expr)) = &v.init else {
        panic!("expected a match expression initializer");
    };
    assert_eq!(match_expr.arms.len(), 3);
}

#[test]
fn lowers_trait_def_and_impl_with_trait() {
    let (ast, errors) = lower(
        r#"
trait Gyao {
  fn gyao(self) -> Int;
  fn guee(aaa: Int) -> Self;
}

impl Nyoee: Gyao {
  fn gyao(self) -> Int { 1 }
  fn guee(aaa: Int) -> Self { Nyoee { x = aaa } }
}
"#,
    );
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    assert_eq!(ast.globals.len(), 2);
    let Globals::TraitDef(trait_def) = &ast.globals[0] else {
        panic!("expected a trait def, got {:?}", ast.globals[0]);
    };
    assert_eq!(trait_def.items.len(), 2);
    assert!(matches!(
        trait_def.items[0].args,
        biwac_ast::TraitItemArgs::Method(_)
    ));
    assert!(matches!(
        trait_def.items[1].args,
        biwac_ast::TraitItemArgs::Assoc(_)
    ));

    let Globals::ImplBlock(impl_block) = &ast.globals[1] else {
        panic!("expected an impl block, got {:?}", ast.globals[1]);
    };
    assert!(impl_block.trait_typ.is_some());
    assert_eq!(impl_block.methods.len(), 1);
    assert_eq!(impl_block.assoc_fns.len(), 1);
}

#[test]
fn lowers_generics_bounds() {
    let (ast, errors) = lower("struct Bbb[T: Gyao] { t: T }");
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    let Globals::TypeDef(TypeDef::Struct(s)) = &ast.globals[0] else {
        panic!("expected a struct def");
    };
    let genargs = s.genargs.as_ref().expect("expected generics decl");
    assert_eq!(genargs.genargs.len(), 1);
    assert_eq!(genargs.genargs[0].bounds.len(), 1);
}

#[test]
fn self_type_in_return_position_lowers_to_self_typ_variant() {
    let src = r#"
impl Duration {
  fn ms(ms: Uint) -> Self {
    Self { ms = ms }
  }
}
"#;
    let (ast, errors) = lower(src);
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    let Globals::ImplBlock(imp) = &ast.globals[0] else {
        panic!("expected impl block, got {:?}", ast.globals[0]);
    };
    let f = &imp.assoc_fns[0];

    // `-> Self` は `TypReprVal::Defined` (単なる識別子) ではなく
    // 専用の `SelfTyp` バリアントでなければならない。
    let biwac_ast::RetTypRepr::Typ(rtype) = &f.rtype else {
        panic!("expected an explicit return type");
    };
    assert!(matches!(rtype.val, biwac_ast::TypReprVal::SelfTyp));

    // `Self { ms = ms }` は struct literal で、path の header が
    // `AbsolutePathHeader::SelfTyp` になり、segments は空。
    let Some(Exprs::Primary(Primary::Literal(biwac_ast::Literal::Struct(lit)))) = &f.expr else {
        panic!("expected a struct literal tail, got {:?}", f.expr);
    };
    assert!(lit.path.segments.is_empty());
    assert!(matches!(
        lit.path.abs_header,
        Some(biwac_ast::AbsolutePathHeader::SelfTyp(_))
    ));
}

#[test]
fn self_colon_colon_call_lowers_to_fn_call_with_self_typ_header() {
    let src = r#"
impl Foo {
  fn make() -> Foo {
    Self::make()
  }
}
"#;
    let (ast, errors) = lower(src);
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    let Globals::ImplBlock(imp) = &ast.globals[0] else {
        panic!("expected impl block, got {:?}", ast.globals[0]);
    };
    let Some(Exprs::Primary(Primary::FnCall(call))) = &imp.assoc_fns[0].expr else {
        panic!("expected a call tail, got {:?}", imp.assoc_fns[0].expr);
    };
    assert!(matches!(
        call.path.abs_header,
        Some(biwac_ast::AbsolutePathHeader::SelfTyp(_))
    ));
    assert_eq!(call.path.segments.len(), 1);
}

#[test]
fn lowers_novel_scene_body_statements() {
    let src = r#"
scene main(g: MyGame) -> MyGame {{
    #let biwa = add(1, 2)
    Hello!
    こんにちは $blue(bold("biwa")) です。 >>
    #if biwa {
        yes
    }
    #biwa = 3
    #endscene g
}}
"#;
    let (ast, errors) = lower(src);
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    assert_eq!(ast.globals.len(), 1);
    let Globals::NovelScene(scene) = &ast.globals[0] else {
        panic!("expected NovelScene, got {:?}", ast.globals[0]);
    };

    // #let biwa = add(1, 2)
    assert!(matches!(scene.stmts[0], biwac_ast::NovelStmt::VarDecl(_)));

    // プレーンなテキスト行が続く (行ごとに分かれていてもよい)。
    let has_plain_text = scene.stmts.iter().any(|s| {
        matches!(
            s,
            biwac_ast::NovelStmt::ContentPush(biwac_ast::NovelContent::Text { .. })
        )
    });
    assert!(has_plain_text, "expected at least one Text content push");

    // `$blue(...)` の埋め込み式。
    let has_embedded_expr = scene.stmts.iter().any(|s| {
        matches!(
            s,
            biwac_ast::NovelStmt::ContentPush(biwac_ast::NovelContent::Expr { .. })
        )
    });
    assert!(
        has_embedded_expr,
        "expected an embedded-expression content push"
    );

    // `>>`
    let has_wait = scene
        .stmts
        .iter()
        .any(|s| matches!(s, biwac_ast::NovelStmt::ContentFlushAndWait(_)));
    assert!(has_wait, "expected a ContentFlushAndWait");

    // #if biwa { yes }
    let if_stmt = scene
        .stmts
        .iter()
        .find_map(|s| match s {
            biwac_ast::NovelStmt::If(i) => Some(i),
            _ => None,
        })
        .expect("expected a NovelIfStmt");
    assert_eq!(if_stmt.then.stmts.len(), 1);
    assert!(if_stmt.els.is_none());
    assert!(matches!(
        if_stmt.then.stmts[0],
        biwac_ast::NovelStmt::ContentPush(biwac_ast::NovelContent::Text { .. })
    ));

    // #biwa = 3
    assert!(
        scene
            .stmts
            .iter()
            .any(|s| matches!(s, biwac_ast::NovelStmt::Assign(_)))
    );

    // #endscene g
    assert!(matches!(
        scene.stmts.last(),
        Some(biwac_ast::NovelStmt::NovelEndScene(_))
    ));
}

#[test]
fn lowers_novel_scene_chara_line_is_skipped() {
    let src = "scene s(g: G) -> G {{\n@biwa\nこんにちは\n}}\n";
    let (ast, errors) = lower(src);
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    let Globals::NovelScene(scene) = &ast.globals[0] else {
        panic!("expected NovelScene");
    };
    // `@biwa` 行自体は捨てられるが、次の行の地の文は残る。
    assert!(
        scene.stmts.iter().any(|s| matches!(
            s,
            biwac_ast::NovelStmt::ContentPush(biwac_ast::NovelContent::Text { .. })
        )),
        "expected the raw text line after @biwa to survive, got {:?}",
        scene.stmts
    );
}

#[test]
fn bare_self_type_used_as_a_value_is_dropped_not_panicking() {
    // 実コンパイラの文法上ありえない書き方 (`Self` 単独を式として使う) だが、
    // biwa-lsp-parser は許容範囲が広いので CST は作れてしまう。
    // Path の segments が空のまま Variable として渡ると `Path::span()` が
    // パニックしうるので、lowering の時点で安全に落とさなければならない。
    let (_ast, errors) = lower("fn f() -> Bool { Self }");
    assert!(!errors.is_empty(), "expected a lowering error, got none");
}
