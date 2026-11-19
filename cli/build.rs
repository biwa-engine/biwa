//! ビルドしたコミットを `BIWA_COMMIT`、ツールチェインの名前を `BIWA_TOOLCHAIN_NAME` として
//! バイナリに焼き込む (`src/toolchain.rs`)。
//!
//! `biwa build` は同じ名前のツールチェインの runtime を使う (`~/.biwa/toolchains/<name>/runtime/`)。
//! CLI と runtime を必ず同じツールチェインのものにするためである。
//!
//! ツールチェインの名前は、版の release (環境変数 `BIWA_RELEASE_VERSION`、`0.1.0` か `v0.1.0`) なら版、
//! それ以外 (nightly・手元のビルド) は `nightly-<commit>` である (release のタグと同じ)。
//!
//! コミットの決め方 (先にあるものほど優先):
//!   1. 環境変数 `BIWA_COMMIT` (そのまま使う)
//!   2. 環境変数 `GITHUB_SHA` (GitHub Actions) の先頭 12 文字
//!   3. `git rev-parse --short=12 HEAD`。追跡しているファイルに未コミットの変更があれば `-dirty` を付ける
//!   4. どれも無ければ `unknown`

use std::path::Path;
use std::process::Command;

/// コミットの名前の長さ (GitHub の短いハッシュより少し長く、衝突しにくくする)。
const SHORT: usize = 12;

fn main() {
    // std はサブモジュール (`library/std`) で、CLI に埋め込む (`src/assets.rs`)。取り忘れると空の std を
    // 抱えた CLI ができ、利用者の手元で初めて壊れていると分かるので、ここで止める。
    let std_lib = Path::new(env!("CARGO_MANIFEST_DIR")).join("../library/std/src/lib.biwa");
    assert!(
        std_lib.is_file(),
        "{} is missing; run `git submodule update --init` (std is embedded into the CLI)",
        std_lib.display()
    );

    println!("cargo:rerun-if-env-changed=BIWA_COMMIT");
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");

    let commit = std::env::var("BIWA_COMMIT")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| {
            std::env::var("GITHUB_SHA")
                .ok()
                .filter(|v| v.len() >= SHORT)
                .map(|v| v[..SHORT].to_string())
        })
        .or_else(from_git)
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=BIWA_COMMIT={commit}");

    println!("cargo:rerun-if-env-changed=BIWA_RELEASE_VERSION");
    let name = match std::env::var("BIWA_RELEASE_VERSION")
        .ok()
        .filter(|v| !v.is_empty())
    {
        Some(v) => {
            let v = v.strip_prefix('v').unwrap_or(&v).to_string();
            let ok = v.split('.').count() == 3
                && v.split('.')
                    .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
            assert!(ok, "BIWA_RELEASE_VERSION must look like 0.1.0, got `{v}`");
            v
        }
        None => format!("nightly-{commit}"),
    };
    // `biwa --version`: `0.1.0 (<commit>)`、版の無いものは名前そのもの (`nightly-<commit>`)。
    let version = if name.starts_with("nightly-") {
        name.clone()
    } else {
        format!("{name} ({commit})")
    };
    println!("cargo:rustc-env=BIWA_TOOLCHAIN_NAME={name}");
    println!("cargo:rustc-env=BIWA_VERSION_STRING={version}");
}

fn from_git() -> Option<String> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").ok()?;
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .args(args)
            .current_dir(&manifest_dir)
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    };

    // HEAD が動いたとき (コミット・チェックアウト) と、index が変わったとき (add など) に焼き直す。
    // 作業ツリーを編集しただけでは焼き直さないので、`-dirty` はずれることがある (`cargo clean -p biwa` で直る)。
    if let Some(git_dir) = git(&["rev-parse", "--absolute-git-dir"]) {
        let git_dir = Path::new(&git_dir);
        println!("cargo:rerun-if-changed={}", git_dir.join("HEAD").display());
        println!("cargo:rerun-if-changed={}", git_dir.join("index").display());
        if let Some(reference) = git(&["symbolic-ref", "-q", "HEAD"]) {
            println!(
                "cargo:rerun-if-changed={}",
                git_dir.join(reference).display()
            );
        }
    }

    let head = git(&["rev-parse", &format!("--short={SHORT}"), "HEAD"])?;
    let dirty =
        git(&["status", "--porcelain", "--untracked-files=no"]).is_some_and(|s| !s.is_empty());
    Some(if dirty { format!("{head}-dirty") } else { head })
}
