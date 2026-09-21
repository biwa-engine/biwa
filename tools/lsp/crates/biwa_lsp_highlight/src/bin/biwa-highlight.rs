use biwa_lsp_highlight::{TokenType, highlight};
use std::env;
use std::fs;

fn main() {
    let args: Vec<String> = env::args().collect();
    let src = if args.len() >= 2 {
        fs::read_to_string(&args[1]).unwrap_or_else(|e| {
            eprintln!("error: {e}");
            std::process::exit(1);
        })
    } else {
        // stdin から読む
        use std::io::Read;
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s).unwrap();
        s
    };

    let tokens = highlight(&src);

    // ANSI カラーで表示
    print_highlighted(&src, &tokens);
}

fn ansi_color(tt: TokenType) -> &'static str {
    match tt {
        TokenType::Keyword => "\x1b[34;1m",      // bold blue
        TokenType::Type => "\x1b[36;1m",         // bold cyan
        TokenType::Function => "\x1b[33m",       // yellow
        TokenType::Variable => "\x1b[0m",        // default
        TokenType::Parameter => "\x1b[35m",      // magenta
        TokenType::Property => "\x1b[36m",       // cyan
        TokenType::Number => "\x1b[32m",         // green
        TokenType::String => "\x1b[31m",         // red
        TokenType::Comment => "\x1b[90m",        // dark gray
        TokenType::Operator => "\x1b[37m",       // light gray
        TokenType::Namespace => "\x1b[34m",      // blue
        TokenType::Method => "\x1b[33;1m",       // bold yellow
        TokenType::Interface => "\x1b[36;1m",    // bold cyan
        TokenType::NovelText => "\x1b[97;1m",    // bold white
        TokenType::NovelCommand => "\x1b[93m",   // bright yellow
        TokenType::NovelCharacter => "\x1b[95m", // bright magenta
    }
}

const RESET: &str = "\x1b[0m";

fn print_highlighted(src: &str, tokens: &[biwa_lsp_highlight::HighlightToken]) {
    let src_bytes = src.as_bytes();
    let mut pos = 0;

    for tok in tokens {
        // ハイライト対象外の区間はデフォルト色で出力
        if pos < tok.start {
            print!("{}", &src[pos..tok.start]);
        }
        let text = &src[tok.start..tok.end];
        print!("{}{}{}", ansi_color(tok.token_type), text, RESET);
        pos = tok.end;
    }

    // 残りを出力
    if pos < src_bytes.len() {
        print!("{}", &src[pos..]);
    }
}
