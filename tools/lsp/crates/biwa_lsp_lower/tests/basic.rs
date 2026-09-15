use biwa_lsp_parser::parse;
use biwac_ast::{Exprs, Globals, Primary, Stmt, TypeDef};
use biwac_base::{IdentInterner, ModId, ModPath};

fn lower(src: &str) -> (biwac_ast::ModAst, Vec<biwa_lsp_lower::LowerError>) {
    let parse_result = parse(src);
    let root = parse_result.syntax();
    let mut interner = IdentInterner::new();
    biwa_lsp_lower::lower_module(
        ModId::new_in_self(0),
        ModPath::Main,
        &mut interner,
        &root,
    )
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
        panic!("expected `self.x` member access on the left of `*`, got {:?}", bin.left);
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
    assert!(matches!(
        f.expr,
        Some(Exprs::Primary(Primary::Variable(_)))
    ));
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
    assert!(matches!(ast.globals[1], Globals::TypeDef(TypeDef::Struct(_))));
}

#[test]
fn type_alias_is_dropped_with_an_error_until_cst_parses_its_rhs() {
    // biwa-lsp-parser の TypeAliasDef は `type X;` までしか読まず
    // `= <type>` を読まないため、biwac_ast::TypeAlias を組み立てられない。
    let (ast, errors) = lower("type Foo;");
    assert!(ast.globals.is_empty());
    assert!(!errors.is_empty());
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
