use std::{path::Path, process::exit};

use clap::Parser;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// package path (optional)
    #[arg(short, long)]
    package_path: Option<String>,

    /// Rebuild every package, ignoring cached build results.
    ///
    /// 差分ビルドは前回のフィンガープリントとの突き合わせで判定するので、
    /// キャッシュを疑ったときの逃げ道として用意しておく。
    #[arg(short, long)]
    rebuild: bool,

    /// Emit the given intermediate representation instead of the default output.
    ///
    /// rustc と同じく、既定の出力を置き換える。
    /// 中間表現は成果物ではなくキャッシュもされないので、
    /// 指定すると鮮度に関わらず建て直しになる。
    #[arg(long, value_enum)]
    emit: Option<EmitKind>,

    /// Code generation target. `wasm` is the only one (and the default).
    #[arg(long)]
    target: Option<String>,
}

#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
enum EmitKind {
    /// MIR のテキスト表現を `<build dir>/<target>/<package>.biwamir` に書き出す。
    Mir,
}

/// `--target` を解決する。省略したら wasm (唯一のターゲット) である。
fn resolve_target(requested: Option<&str>) -> Result<biwac_base::Target, String> {
    match requested {
        None => Ok(biwac_base::Target::Wasm),
        Some(name) => biwac_base::Target::from_name(name).ok_or_else(|| {
            format!(
                "unknown target `{name}` (available: {})",
                biwac_base::describe_targets(biwac_base::Target::ALL)
            )
        }),
    }
}

fn main() {
    let args = Args::parse();

    let pkg_root_path = Path::new(args.package_path.as_deref().unwrap_or("."));

    if !pkg_root_path.is_dir() {
        panic!(
            "Directory expected, but got file: `{}`",
            pkg_root_path
                .as_os_str()
                .to_str()
                .expect("broken package root path")
        );
    }

    let target = match resolve_target(args.target.as_deref()) {
        Ok(t) => t,
        Err(message) => {
            eprintln!("Error: {message}");
            exit(1);
        }
    };

    let res = biwac_driver::compile(
        pkg_root_path.to_path_buf(),
        biwac_driver::BuildOptions {
            force_rebuild: args.rebuild,
            emit_mir: args.emit == Some(EmitKind::Mir),
            target,
        },
    );

    match res {
        Ok(_) => exit(0),
        Err(_) => exit(1),
    }
}
