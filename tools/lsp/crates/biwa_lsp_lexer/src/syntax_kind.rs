/// すべての構文要素を表す kind。rowan の Language::Kind に対応する u16 newtype。
/// 通常モードとノベルモードの両方のトークンを1つの enum にまとめる。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
pub enum SyntaxKind {
    // ── trivia ──────────────────────────────────────────────────────────────
    Whitespace,
    Newline,
    LineComment,

    // ── literals ────────────────────────────────────────────────────────────
    IntLiteral,
    FloatLiteral,
    StringLiteral,
    TrueLiteral,
    FalseLiteral,
    NoneLiteral,

    // ── keywords ────────────────────────────────────────────────────────────
    KwImport,
    KwAs,
    KwFn,
    KwStruct,
    KwType,
    KwImpl,
    KwScene,
    KwLet,
    KwIf,
    KwElse,
    KwWhile,
    KwFor,
    KwIn,
    KwSelf,
    KwReturn,
    KwPackage,

    // ── built-in types ───────────────────────────────────────────────────────
    KwVoid,
    KwInt,
    KwUint,
    KwFloat,
    KwBool,

    // ── identifiers ─────────────────────────────────────────────────────────
    Ident,

    // ── punctuation ─────────────────────────────────────────────────────────
    Semi,         // ;
    Colon,        // :
    ColonColon,   // ::
    Comma,        // ,
    Dot,          // .
    Arrow,        // ->
    FatArrow,     // =>
    Eq,           // =
    EqEq,         // ==
    BangEq,       // !=
    Bang,         // !
    Plus,         // +
    Minus,        // -
    Star,         // *
    Slash,        // /
    Percent,      // %
    Lt,           // <
    LtEq,         // <=
    Gt,           // >
    GtEq,         // >=
    AmpAmp,       // &&
    PipePipe,     // ||
    LParen,       // (
    RParen,       // )
    LBrace,       // {
    RBrace,       // }
    LBracket,     // [
    RBracket,     // ]
    DoubleLBrace, // {{  (novel mode open)
    DoubleRBrace, // }}  (novel mode close, only valid at line-start)

    // ── novel mode tokens ────────────────────────────────────────────────────
    NovelText,        // プレーンなテキスト行
    NovelAt,          // @ (キャラクター指定)
    NovelHash,        // # (コマンド行)
    NovelDollarBrace, // ${ (値埋め込み開始)
    NovelCloseBrace,  // } (値埋め込み終了)

    // ── special ─────────────────────────────────────────────────────────────
    Error,
    Eof,

    // ── composite nodes (leaf でない CST ノード) ─────────────────────────────
    // parser が使用する。lexer は生成しない。
    Root,
    ImportDecl,
    FunctionDef,
    MethodDef,
    StructDef,
    TypeAliasDef,
    ImplBlock,
    SceneDef,
    NovelMode,
    NovelModeBody,
    FunctionArgDecl,
    MethodArgDecl,
    GenericsArgDecl,
    GenericsArgList,
    TypeRepr,
    BlockStmt,
    BlockExpr,
    ExprStmt,
    VarDefStmt,
    AssignStmt,
    IfStmt,
    WhileStmt,
    ForStmt,
    IfExpr,
    BinaryExpr,
    UnaryExpr,
    PostfixExpr,
    CallArgList,
    PrimaryExpr,
    StructLiteral,
    StructLiteralField,
    IdentPath,
    Literal,
    ParenExpr,
}

impl From<SyntaxKind> for rowan::SyntaxKind {
    fn from(kind: SyntaxKind) -> Self {
        rowan::SyntaxKind(kind as u16)
    }
}
