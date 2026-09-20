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
    KwEnum,
    KwMatch,
    KwTrait,
    /// `_` 単体のみ。`_probe` のような識別子には影響しない
    /// (biwac_lexer と同じ扱い。`docs/enum-and-match.md` 参照)。
    KwUnderscore,

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
    NovelText,   // プレーンなテキスト行
    NovelAt,     // @ (キャラクター指定)
    NovelHash,   // # (コマンド行の導入記号。中身は通常コードのトークンで続く)
    NovelDollar, // $ (埋め込み式の導入記号。中身は通常コードのトークンで続く)
    NovelWait,   // >> (積んだ内容をまとめて出してクリックを待つ)

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
    EnumDef,
    VariantDecl,
    TraitDef,
    TraitItemDecl,
    TypeAliasDef,
    ImplBlock,
    SceneDef,
    NovelMode,
    NovelModeBody,
    /// ノベルモードの `#` コマンド行。中身 (`#` の次から、継続が終わるまで) は
    /// 通常コードの文と同じ子ノード/トークンで構成される。
    NovelCommandLine,
    /// ノベルモードの `$` 埋め込み式。`docs/content-api.md` の EBNF に対応する。
    NovelEmbeddedExpr,
    FunctionArgDecl,
    MethodArgDecl,
    GenericsArgDecl,
    GenericsArgItem,
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
    MatchStmt,
    MatchExpr,
    MatchArm,
    /// パターン。`_`、識別子/バリアントパス、`Path(..)`、`Path { .. }`。
    Pattern,
    PatternTupleFields,
    PatternStructFields,
    PatternField,
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
