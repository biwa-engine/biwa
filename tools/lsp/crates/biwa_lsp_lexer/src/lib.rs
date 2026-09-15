pub mod lexer;
pub mod pre_scan;
pub mod syntax_kind;

pub use lexer::{Token, lex};
pub use pre_scan::{Segment, pre_scan};
pub use syntax_kind::SyntaxKind;
