use std::cell::OnceCell;

use biwac_base::{IdentInterner, ModId, ModPath};
use biwac_span::Span;

use biwac_ast::{
    Exprs, Globals, Ident, IntegerLiteral, Literal, Primary, Stmt, StringLiteral, TypDecl, VarDecl,
};

#[test]
fn test1() {
    let modpath = ModPath::Main;
    let mod_id = ModId::new_in_self(0);
    let mut interner = IdentInterner::new();

    // NOTE: Rustの生文字列の扱いでは以下の場合
    // 空文字列の0行目が含まれ、fnは1行目となるため注意
    let src = r#"
fn foo() {
    let x = 0;
    // comment "
    let str = "string";
}
"#;

    let tokens = biwac_lexer::lex(&mut interner, mod_id, src).unwrap();

    let module = crate::Parser::new(mod_id, modpath, tokens, &mut interner)
        .try_parse()
        .unwrap();

    let g0 = module.globals.first().unwrap();

    let fn_foo = if let Globals::FnDef(f) = g0 {
        f
    } else {
        panic!("not a function: {g0:#?}");
    };

    assert_eq!(2, fn_foo.stmts.len());
    assert_eq!(
        &Stmt::VarDecl(VarDecl {
            typ: TypDecl::Any,
            id: Ident {
                id: interner.get_or_insert("x"),
                span: Span::new(mod_id, 20, 21)
            },
            init: Exprs::Primary(Primary::Literal(Literal::Integer(IntegerLiteral {
                val: 0,
                span: Span::new(mod_id, 24, 25)
            }))),
            span: Span::new(mod_id, 16, 26),
            var_id: OnceCell::new()
        }),
        fn_foo.stmts.first().unwrap()
    );
    assert_eq!(
        &Stmt::VarDecl(VarDecl {
            typ: TypDecl::Any,
            id: Ident {
                id: interner.get_or_insert("str"),
                span: Span::new(mod_id, 52, 55)
            },
            init: Exprs::Primary(Primary::Literal(Literal::String(StringLiteral {
                val: "string".to_string(),
                span: Span::new(mod_id, 58, 66)
            }))),
            span: Span::new(mod_id, 48, 67),
            var_id: OnceCell::new()
        }),
        fn_foo.stmts.get(1).unwrap()
    );
    assert_eq!(None, fn_foo.expr);
}

/// `if`/`while` の条件式に構造体リテラルを置けないことの回帰テスト。
///
/// 条件式の直後にはブロックの `{` が来るので、
/// `if flag {` の `{` を構造体リテラルの開始と読むと必ず誤る。
mod no_struct_literal_in_condition {
    use biwac_ast::{Exprs, FnDef, Globals, Literal, Primary, Stmt};
    use biwac_base::{IdentInterner, ModId, ModPath};

    fn parse_fn(src: &str) -> FnDef {
        let mod_id = ModId::new_in_self(0);
        let mut interner = IdentInterner::new();

        let tokens = biwac_lexer::lex(&mut interner, mod_id, src).unwrap();
        let module = crate::Parser::new(mod_id, ModPath::Main, tokens, &mut interner)
            .try_parse()
            .unwrap_or_else(|e| panic!("parse failed: {e:#?}"));

        match module.globals.into_iter().next().unwrap() {
            Globals::FnDef(f) => f,
            g => panic!("not a function: {g:#?}"),
        }
    }

    fn is_struct_literal(expr: &Exprs) -> bool {
        matches!(expr, Exprs::Primary(Primary::Literal(Literal::Struct(_))))
    }

    /// 裸の識別子を条件に書ける。`{` はブロックの開始として読まれる。
    #[test]
    fn bare_identifier_is_a_condition_not_a_struct_literal() {
        let f = parse_fn(
            r#"
fn foo(flag: Bool) {
    if flag {
        let x = 0;
    }
}
"#,
        );

        let Stmt::If(if_stmt) = f.stmts.first().unwrap() else {
            panic!("not an if: {:#?}", f.stmts);
        };

        assert!(
            matches!(if_stmt.cond, Exprs::Primary(Primary::Variable(_))),
            "condition should be a variable, got {:#?}",
            if_stmt.cond
        );
        assert_eq!(1, if_stmt.then.stmts.len());
    }

    /// `while` の条件式も同じ扱いになる。
    #[test]
    fn bare_identifier_is_a_while_condition() {
        let f = parse_fn(
            r#"
fn foo(flag: Bool) {
    while flag {
        let x = 0;
    }
}
"#,
        );

        let Stmt::While(while_stmt) = f.stmts.first().unwrap() else {
            panic!("not a while: {:#?}", f.stmts);
        };

        assert!(
            matches!(while_stmt.cond, Exprs::Primary(Primary::Variable(_))),
            "condition should be a variable, got {:#?}",
            while_stmt.cond
        );
        assert_eq!(1, while_stmt.stmts.stmts.len());
    }

    /// 括弧の内側では意味が閉じるので、制限は解ける。
    #[test]
    fn parentheses_lift_the_restriction() {
        let f = parse_fn(
            r#"
fn foo() {
    if (Flagged { on = TRUE }).on {
        let x = 0;
    }
}
"#,
        );

        let Stmt::If(if_stmt) = f.stmts.first().unwrap() else {
            panic!("not an if: {:#?}", f.stmts);
        };

        let Exprs::Primary(Primary::MemberAccess(access)) = &if_stmt.cond else {
            panic!("not a member access: {:#?}", if_stmt.cond);
        };
        assert!(
            is_struct_literal(&access.left),
            "the parenthesized expression should be a struct literal, got {:#?}",
            access.left
        );
    }

    /// 引数リストの内側でも制限は解ける。
    #[test]
    fn arguments_lift_the_restriction() {
        let f = parse_fn(
            r#"
fn foo() {
    if takes(Flagged { on = TRUE }) {
        let x = 0;
    }
}
"#,
        );

        let Stmt::If(if_stmt) = f.stmts.first().unwrap() else {
            panic!("not an if: {:#?}", f.stmts);
        };

        let Exprs::Primary(Primary::Call(call)) = &if_stmt.cond else {
            panic!("not a call: {:#?}", if_stmt.cond);
        };
        assert!(
            is_struct_literal(call.args.first().unwrap()),
            "the argument should be a struct literal, got {:#?}",
            call.args
        );
    }

    /// 条件式を抜ければ元に戻る。ブロックの中では構造体リテラルを書ける。
    #[test]
    fn restriction_ends_with_the_condition() {
        let f = parse_fn(
            r#"
fn foo(flag: Bool) {
    if flag {
        let f = Flagged { on = flag };
    }
}
"#,
        );

        let Stmt::If(if_stmt) = f.stmts.first().unwrap() else {
            panic!("not an if: {:#?}", f.stmts);
        };

        let Stmt::VarDecl(decl) = if_stmt.then.stmts.first().unwrap() else {
            panic!("not a let: {:#?}", if_stmt.then.stmts);
        };
        assert!(
            is_struct_literal(&decl.init),
            "the initializer should be a struct literal, got {:#?}",
            decl.init
        );
    }
}

/// enum 宣言と match のパース。
mod enum_and_match {
    use biwac_ast::{
        EnumDef, Globals, MatchExprArm, Pattern, PatternFields, Primary, Stmt, TypeDef,
        VariantFieldsDecl,
    };
    use biwac_base::{IdentInterner, ModId, ModPath};

    fn parse(src: &str) -> (Vec<Globals>, IdentInterner) {
        let mod_id = ModId::new_in_self(0);
        let mut interner = IdentInterner::new();

        let tokens = biwac_lexer::lex(&mut interner, mod_id, src).unwrap();
        let module = crate::Parser::new(mod_id, ModPath::Main, tokens, &mut interner)
            .try_parse()
            .unwrap_or_else(|e| panic!("parse failed: {e:#?}"));

        (module.globals, interner)
    }

    fn enum_def(globals: Vec<Globals>) -> EnumDef {
        match globals.into_iter().next().unwrap() {
            Globals::TypeDef(TypeDef::Enum(e)) => e,
            g => panic!("not an enum: {g:#?}"),
        }
    }

    #[test]
    fn three_shapes_of_variant() {
        let (globals, interner) = parse(
            r#"
enum Color {
    Red,
    Rgb(Int, Int, Int),
    Named { name: Int, alpha: Int },
}
"#,
        );

        let e = enum_def(globals);
        assert_eq!("Color", interner.get_str(&e.id.id).unwrap());
        assert_eq!(3, e.variants.len());

        assert!(matches!(e.variants[0].fields, VariantFieldsDecl::Unit));
        match &e.variants[1].fields {
            VariantFieldsDecl::Tuple(typs) => assert_eq!(3, typs.len()),
            other => panic!("not a tuple variant: {other:#?}"),
        }
        match &e.variants[2].fields {
            VariantFieldsDecl::Struct(members) => assert_eq!(2, members.len()),
            other => panic!("not a struct variant: {other:#?}"),
        }
    }

    #[test]
    fn generic_enum() {
        let (globals, _) = parse("enum Option[T] { None, Some(T), }");

        let e = enum_def(globals);
        assert_eq!(1, e.genargs.as_ref().unwrap().genargs.len());
        assert_eq!(2, e.variants.len());
    }

    /// アームが値を返すなら match は式になる。
    #[test]
    fn match_as_expression() {
        let (globals, _) = parse(
            r#"
fn f(o: Int) -> Int {
    let n = match o {
        Option::None => 0,
        Option::Some(x) => x,
    };
    n
}
"#,
        );

        let Globals::FnDef(f) = globals.into_iter().next().unwrap() else {
            panic!("not a function");
        };
        let Stmt::VarDecl(decl) = f.stmts.first().unwrap() else {
            panic!("not a let: {:#?}", f.stmts);
        };
        let biwac_ast::Exprs::Primary(Primary::Match(m)) = &decl.init else {
            panic!("not a match expression: {:#?}", decl.init);
        };

        assert_eq!(2, m.arms.len());
        assert_variant_pattern(&m.arms[0], PatternShape::Unit);
        assert_variant_pattern(&m.arms[1], PatternShape::Tuple(1));
    }

    /// アームがブロック文なら match は文になる。
    #[test]
    fn match_as_statement() {
        let (globals, _) = parse(
            r#"
fn f(c: Int) {
    match c {
        Color::Red => { let x = 0; }
        Color::Named { name = n, alpha } => { let y = 0; }
        _ => { let z = 0; }
    }
}
"#,
        );

        let Globals::FnDef(f) = globals.into_iter().next().unwrap() else {
            panic!("not a function");
        };
        let Stmt::Match(m) = f.stmts.first().unwrap() else {
            panic!("not a match statement: {:#?}", f.stmts);
        };

        assert_eq!(3, m.arms.len());

        match &m.arms[1].pattern {
            Pattern::Variant(v) => match &v.fields {
                // 省略形の `alpha` も同名への束縛として入っている。
                PatternFields::Struct(fields) => {
                    assert_eq!(2, fields.len());
                    assert!(matches!(fields[1].1, Pattern::Ident(_)));
                }
                other => panic!("not a struct pattern: {other:#?}"),
            },
            other => panic!("not a variant pattern: {other:#?}"),
        }

        assert!(matches!(m.arms[2].pattern, Pattern::Wildcard(_)));
    }

    /// 単独の識別子は束縛としてパースされる。
    /// バリアントかどうかは名前解決が決める。
    #[test]
    fn bare_identifier_is_a_binding() {
        let (globals, _) = parse("fn f(c: Int) { match c { x => { let y = 0; } } }");

        let Globals::FnDef(f) = globals.into_iter().next().unwrap() else {
            panic!("not a function");
        };
        let Stmt::Match(m) = f.stmts.first().unwrap() else {
            panic!("not a match statement");
        };
        assert!(matches!(m.arms[0].pattern, Pattern::Ident(_)));
    }

    enum PatternShape {
        Unit,
        Tuple(usize),
    }

    fn assert_variant_pattern(arm: &MatchExprArm, shape: PatternShape) {
        let Pattern::Variant(v) = &arm.pattern else {
            panic!("not a variant pattern: {:#?}", arm.pattern);
        };
        match (&v.fields, shape) {
            (PatternFields::Unit, PatternShape::Unit) => {}
            (PatternFields::Tuple(pats), PatternShape::Tuple(n)) => assert_eq!(n, pats.len()),
            (other, _) => panic!("unexpected pattern fields: {other:#?}"),
        }
    }
}

/// 関数型と、任意の式を呼び先にした呼び出し
/// (`docs/function-as-the-first-class-type-impl-status.md` のステップ 1・2)。
mod first_class_fn {
    use biwac_ast::{Exprs, FnDef, Globals, Primary, TypReprVal};
    use biwac_base::{IdentInterner, ModId, ModPath};

    fn parse_fn(src: &str) -> FnDef {
        let mod_id = ModId::new_in_self(0);
        let mut interner = IdentInterner::new();

        let tokens = biwac_lexer::lex(&mut interner, mod_id, src).unwrap();
        let module = crate::Parser::new(mod_id, ModPath::Main, tokens, &mut interner)
            .try_parse()
            .unwrap_or_else(|e| panic!("parse failed: {e:#?}"));

        match module.globals.into_iter().next().unwrap() {
            Globals::FnDef(f) => f,
            g => panic!("not a function: {g:#?}"),
        }
    }

    #[test]
    fn fn_type_in_argument() {
        let f = parse_fn("fn apply(f: fn(Int, Bool) -> Int, g: fn(Int)) {}");
        let TypReprVal::Fn(f_ty) = &f.args.args[0].typ.val else {
            panic!("not a fn type: {:#?}", f.args.args[0].typ);
        };
        assert_eq!(f_ty.args.len(), 2);
        assert!(f_ty.rty.is_some());
        let TypReprVal::Fn(g_ty) = &f.args.args[1].typ.val else {
            panic!("not a fn type: {:#?}", f.args.args[1].typ);
        };
        assert!(g_ty.rty.is_none(), "an omitted return type is Void");
    }

    /// 呼び出しの結果をさらに呼べる。後置演算子なので左結合。
    #[test]
    fn call_on_a_call() {
        let f = parse_fn("fn foo() -> Int { make()(1)(2) }");
        let Some(Exprs::Primary(Primary::Call(outer))) = &f.expr else {
            panic!("not a call: {:#?}", f.expr);
        };
        // `make()(1)(2)` = ((make)())(1))(2)
        let Exprs::Primary(Primary::Call(middle)) = outer.callee.as_ref() else {
            panic!("not a nested call: {:#?}", outer.callee);
        };
        let Exprs::Primary(Primary::Call(inner)) = middle.callee.as_ref() else {
            panic!("not a nested call: {:#?}", middle.callee);
        };
        assert!(inner.args.is_empty());
        assert!(matches!(
            inner.callee.as_ref(),
            Exprs::Primary(Primary::Variable(_))
        ));
    }

    /// 括弧で包んだ式も呼び先にできる。
    #[test]
    fn call_on_a_parenthesized_expression() {
        let f = parse_fn("fn foo(f: fn(Int) -> Int) -> Int { (f)(1) }");
        assert!(matches!(&f.expr, Some(Exprs::Primary(Primary::Call(_)))));
    }

    /// `x.bar(..)` は「メンバアクセス `x.bar` の呼び出し」として読む。
    /// メソッドかメンバ (関数型) の値の呼び出しかは型推論が決める。
    #[test]
    fn dot_call_is_a_call_on_a_member_access() {
        let f = parse_fn("fn foo(b: Button) { b.on_click(1) }");
        let Some(Exprs::Primary(Primary::Call(call))) = &f.expr else {
            panic!("not a call: {:#?}", f.expr);
        };
        assert!(matches!(
            call.callee.as_ref(),
            Exprs::Primary(Primary::MemberAccess(_))
        ));
    }

    /// 無名関数。引数の型と戻り値の型は省略できる。
    #[test]
    fn fn_literal() {
        let f = parse_fn("fn foo() -> Int { fn(x, y: Int) -> Int { x + y }(1, 2) }");
        let Some(Exprs::Primary(Primary::Call(call))) = &f.expr else {
            panic!("not a call: {:#?}", f.expr);
        };
        let Exprs::Primary(Primary::FnLiteral(lit)) = call.callee.as_ref() else {
            panic!("not a fn literal: {:#?}", call.callee);
        };
        assert_eq!(lit.args.len(), 2);
        assert!(lit.args[0].typ.is_none());
        assert!(lit.args[1].typ.is_some());
        assert!(lit.rtype.is_some());
        assert!(lit.expr.is_some());

        // 本体に文を並べられる。値を返さない本体もある。
        let f = parse_fn("fn foo() { apply(fn(x) { let y = x; print(y); }) }");
        assert!(f.expr.is_some());
    }

    /// `Self::new` はパスの値で、`(..)` が続けば呼び出しになる。
    #[test]
    fn self_path_is_a_value() {
        let f = parse_fn("fn foo() -> Int { Self::new(1) }");
        let Some(Exprs::Primary(Primary::Call(call))) = &f.expr else {
            panic!("not a call: {:#?}", f.expr);
        };
        assert!(matches!(
            call.callee.as_ref(),
            Exprs::Primary(Primary::Variable(_))
        ));
        let f = parse_fn("fn foo() -> Int { Self::new }");
        assert!(matches!(
            &f.expr,
            Some(Exprs::Primary(Primary::Variable(_)))
        ));
    }
}

/// 可視性・`mod` 宣言・`super::` パスの構文 (issue #8 の段階 1)。
///
/// 段階 1 では受理するだけで検査はしない。ここでは書ける場所で読めること、
/// 書けない場所 (variant・trait の項目・trait impl の項目など) で拒否されることを確かめる。
mod visibility {
    use biwac_ast::{
        AbsolutePathHeader, Exprs, Globals, ModAst, Primary, TypeDef, Variable, Visibility,
    };
    use biwac_base::{IdentInterner, ModId, ModPath};

    use crate::ParseError;

    fn parse(src: &str) -> Result<ModAst, String> {
        let mod_id = ModId::new_in_self(0);
        let mut interner = IdentInterner::new();
        let tokens = biwac_lexer::lex(&mut interner, mod_id, src).unwrap();
        crate::Parser::new(mod_id, ModPath::Main, tokens, &mut interner)
            .try_parse()
            .map_err(|e| format!("{e:?}"))
    }

    fn is_not_allowed(src: &str, place_contains: &str) -> bool {
        let mod_id = ModId::new_in_self(0);
        let mut interner = IdentInterner::new();
        let tokens = biwac_lexer::lex(&mut interner, mod_id, src).unwrap();
        match crate::Parser::new(mod_id, ModPath::Main, tokens, &mut interner).try_parse() {
            Err(ParseError::NotAllowedHere { place, .. }) => place.contains(place_contains),
            other => panic!("expected NotAllowedHere, got {other:?}"),
        }
    }

    fn kind(v: &Visibility) -> &'static str {
        match v {
            Visibility::Private => "private",
            Visibility::Super(_) => "super",
            Visibility::Package(_) => "package",
            Visibility::Public(_) => "pub",
        }
    }

    #[test]
    fn accepts_visibility_where_allowed() {
        let ast = parse(
            r#"
mod a;
pub mod b;
pub fn f() {}
pub(super) fn g() {}
pub(package) struct S { pub x: Int, pub(super) y: Int, z: Int }
pub enum E { A, B(Int) }
pub type T = Int;
pub trait Tr { fn t(self); }
impl S {
  pub fn new() -> Self { S { x = 0, y = 0, z = 0 } }
  pub(package) fn get(self) -> Int { self.x }
}
"#,
        )
        .unwrap();

        let mut seen = Vec::new();
        for g in &ast.globals {
            match g {
                Globals::Mod(m) => seen.push(kind(&m.vis)),
                Globals::FnDef(f) => seen.push(kind(&f.vis)),
                Globals::TypeDef(TypeDef::Struct(s)) => {
                    seen.push(kind(&s.vis));
                    seen.extend(s.members.iter().map(|m| kind(&m.vis)));
                }
                Globals::TypeDef(TypeDef::Enum(e)) => seen.push(kind(&e.vis)),
                Globals::TypeDef(TypeDef::TypeAlias(t)) => seen.push(kind(&t.vis)),
                Globals::TraitDef(t) => seen.push(kind(&t.vis)),
                Globals::ImplBlock(b) => {
                    seen.extend(b.assoc_fns.iter().map(|f| kind(&f.vis)));
                    seen.extend(b.methods.iter().map(|m| kind(&m.vis)));
                }
                _ => {}
            }
        }
        assert_eq!(
            seen,
            [
                "private", "pub", "pub", "super", "package", "pub", "super", "private", "pub",
                "pub", "pub", "pub", "package"
            ]
        );
    }

    #[test]
    fn rejects_visibility_where_not_allowed() {
        assert!(is_not_allowed("enum E { pub A }", "enum variant"));
        assert!(is_not_allowed(
            "enum E { A(pub Int) }",
            "field of an enum variant"
        ));
        assert!(is_not_allowed(
            "enum E { A { pub x: Int } }",
            "field of an enum variant"
        ));
        assert!(is_not_allowed(
            "trait T { pub fn t(self); }",
            "item of a trait"
        ));
        assert!(is_not_allowed(
            "trait T { fn t(self); } impl Int: T { pub fn t(self) {} }",
            "item of a trait impl"
        ));
        assert!(is_not_allowed("pub impl Int {}", "impl block"));
        assert!(is_not_allowed("pub import package::a;", "import"));
    }

    #[test]
    fn rejects_attribute_on_mod() {
        assert!(matches!(
            parse("[[native]] mod a;"),
            Err(e) if e.contains("NotAllowedHere")
        ));
    }

    #[test]
    fn parses_super_paths() {
        let ast = parse(
            r#"
import super::super::a::b;
fn f() -> super::T { super::g() }
"#,
        )
        .unwrap();

        let Globals::Import(import) = &ast.globals[0] else {
            panic!("not an import")
        };
        assert!(matches!(
            import.path.abs_header,
            Some(AbsolutePathHeader::Super { depth: 2, .. })
        ));
        assert_eq!(import.path.segments.len(), 2);

        let Globals::FnDef(f) = &ast.globals[1] else {
            panic!("not a function")
        };
        let biwac_ast::RetTypRepr::Typ(rt) = &f.rtype else {
            panic!("no return type")
        };
        let biwac_ast::TypReprVal::Defined(def) = &rt.val else {
            panic!("not a defined type")
        };
        assert!(matches!(
            def.path.abs_header,
            Some(AbsolutePathHeader::Super { depth: 1, .. })
        ));
        let Some(Exprs::Primary(Primary::Call(call))) = &f.expr else {
            panic!("not a call: {:?}", f.expr)
        };
        let Exprs::Primary(Primary::Variable(Variable::Path(p))) = &*call.callee else {
            panic!("callee is not a path")
        };
        assert!(matches!(
            p.abs_header,
            Some(AbsolutePathHeader::Super { depth: 1, .. })
        ));
    }
}
