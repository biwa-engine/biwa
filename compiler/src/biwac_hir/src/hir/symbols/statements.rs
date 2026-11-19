use biwac_span::{Span, VarId};

use crate::{Expr, Ident, Pattern, Primary, Ty};

#[derive(Debug, Clone)]
pub struct DecledVar {
    pub id: Ident,
    pub ty: Ty, // if not type annotated, Ty::Infer(InferTy)
}

#[derive(Debug, Clone)]
pub struct BlockStmt {
    pub stmts: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct IfStmt {
    pub cond: Expr,
    pub then: BlockStmt,
    pub els: Option<BlockStmt>,
}

/// `match` の文形。アームの本体はブロック文である。
#[derive(Debug, Clone)]
pub struct MatchStmt {
    pub scrutinee: Expr,
    pub arms: Vec<MatchStmtArm>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct MatchStmtArm {
    pub pattern: Pattern,
    pub body: BlockStmt,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct WhileStmt {
    pub cond: Expr,
    pub stmts: BlockStmt,
}

#[derive(Debug, Clone)]
pub struct VarDecl {
    pub id: VarId,
    pub init: Expr,
}

#[derive(Debug, Clone)]
pub struct ExprStmt {
    pub expr: Expr,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ReturnStmt {
    pub expr: Expr,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct AssignStmt {
    pub dst: Primary,
    pub src: Expr,
    pub span: Span,
}

// novel-specific statements

/// novel statement が展開された syscall の発行。
///
/// 中身は普通の呼び出し式で、型推論・MIR 構築は普通の呼び出しとして扱う。
/// 区別して残しているのは、novel statement から来たことを LSP などが見分けられるようにするためである。
/// (かつての TypeScript のターゲットは、ここに `yield` を置いて scene を中断させていた。
/// wasm ではブロッキング syscall がホスト側で待つので、コンパイラは中断を作らない。
/// `docs/execution-model.md` を参照)
#[derive(Debug, Clone)]
pub struct NovelSyscallStmt {
    pub call: Expr,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Block(BlockStmt),
    Expr(ExprStmt),
    Return(ReturnStmt),
    If(IfStmt),
    Match(MatchStmt),
    While(WhileStmt),
    VarDecl(VarDecl),
    Assign(AssignStmt),
    /// novel statement (生テキスト・`$` の埋め込み式・`>>`) の展開先。
    NovelSyscall(NovelSyscallStmt),
}
