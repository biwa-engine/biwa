//! `biwa dev`: 開発サーバ。
//!
//! やることは 3 つ。
//!   1. 同梱物 (エンジン・std) をプロジェクトに揃える
//!   2. biwac でビルドし、生成物をエンジンから見える場所へ移す
//!   3. Vite を起動し、`src/` の `.biwa` の変更を拾って 2 を繰り返す

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use biwac_base::Target;
use clap::Args;
use notify::{RecursiveMode, Watcher};

use crate::project::Project;
use crate::runtime;

/// 変更イベントが落ち着くのを待つ時間。
/// エディタは保存 1 回で複数のイベントを出すので、まとめてから 1 回だけ建て直す。
const DEBOUNCE: Duration = Duration::from_millis(120);

#[derive(Debug, Args)]
pub struct DevArgs {
    /// 対象の biwa パッケージ (省略時はカレントディレクトリから探す)
    #[arg(short, long, value_name = "DIR")]
    pub package_path: Option<PathBuf>,

    /// キャッシュを無視して最初から建て直す
    #[arg(short, long)]
    pub rebuild: bool,

    /// 開発サーバのポート (省略時は Vite の既定値)
    #[arg(long)]
    pub port: Option<u16>,

    /// コード生成のターゲット
    ///
    /// 既定は `wasm`。生成物は Worker で走り、エンジンの API は
    /// ホスト関数の呼び出し (= syscall) になる。
    /// `typescript` を選ぶと、scene が generator として出力され、
    /// エンジンのメインスレッドで動く従来の経路になる。
    #[arg(long, default_value = DEFAULT_TARGET)]
    pub target: String,
}

/// `--target` の既定値。
const DEFAULT_TARGET: &str = "wasm";

/// `--target` を解決する。
///
/// 名前が存在するかと、このビルドの biwac が実際に生成できるかは別である。
/// 前者は `Target::from_name`、後者は `available_targets` が答える。
fn resolve_target(name: &str) -> Result<Target> {
    let available = biwac_generator::available_targets();

    match Target::from_name(name) {
        Some(t) if available.contains(&t) => Ok(t),
        Some(t) => bail!(
            "target `{t}` is not available in this build of biwa (available: {})",
            biwac_base::describe_targets(&available)
        ),
        None => bail!(
            "unknown target `{name}` (available: {})",
            biwac_base::describe_targets(&available)
        ),
    }
}

pub fn run(args: DevArgs) -> Result<()> {
    let start_dir = args
        .package_path
        .clone()
        .unwrap_or_else(|| PathBuf::from("."));
    let project = Project::discover(&start_dir)?;
    let target = resolve_target(&args.target)?;

    println!("Package: {} ({})", project.name, project.root.display());
    println!("Target: {target}");

    runtime::ensure_std(&project)?;
    runtime::ensure_engine(&project)?;
    // エンジンを展開したあとに張る。展開は `.biwa_runtime/` を掃除しないので、
    // 次回以降エンジンが更新されてもこのリンクは残る。
    runtime::link_assets(&project)?;

    // 初回は通らないと開発サーバを立てる意味がないので、失敗したらそこで止める。
    build(&project, target, args.rebuild)?;

    let mut vite = ViteProcess::spawn(&project, args.port)?;

    watch_loop(&project, target, &mut vite)
}

/// biwac を呼んで、生成物をエンジン側へ配る。
fn build(project: &Project, target: Target, force_rebuild: bool) -> Result<()> {
    compile(project, target, force_rebuild)?;
    runtime::sync_generated(project, target)
}

/// コンパイラ本体の呼び出し。
///
/// biwac はライブラリとして呼ぶ (`biwac` コマンドは経由しない)。
/// まだ未実装のエラー表示経路で panic することがあるので、
/// 開発サーバごと落ちないようにここで受け止める。
fn compile(project: &Project, target: Target, force_rebuild: bool) -> Result<()> {
    let root = project.root.clone();

    let result = std::panic::catch_unwind(move || {
        biwac_driver::compile(
            root,
            biwac_driver::BuildOptions {
                force_rebuild,
                // 中間表現は開発サーバの関心事ではない。
                emit_mir: false,
                target,
            },
        )
    });

    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(())) => bail!("build failed"),
        Err(_) => bail!("the compiler panicked while building this package"),
    }
}

/// `src/` を監視して、`.biwa` が変わるたびに建て直す。
///
/// Vite が落ちたらこちらも終わる。
fn watch_loop(project: &Project, target: Target, vite: &mut ViteProcess) -> Result<()> {
    let (tx, rx) = mpsc::channel();

    let mut watcher = notify::recommended_watcher(move |res| {
        // 受け手が居なくなっている (終了処理中) 場合は捨てる。
        let _ = tx.send(res);
    })
    .context("failed to start the file watcher")?;

    for dir in runtime::watch_targets(project) {
        watcher
            .watch(&dir, RecursiveMode::Recursive)
            .with_context(|| format!("failed to watch {}", dir.display()))?;
        println!("Watching {}", dir.display());
    }

    loop {
        if let Some(status) = vite.try_wait()? {
            if status.success() {
                return Ok(());
            }
            bail!("the vite dev server exited with {status}");
        }

        let event = match rx.recv_timeout(Duration::from_millis(300)) {
            Ok(Ok(event)) => event,
            Ok(Err(e)) => {
                eprintln!("warning: file watcher error: {e}");
                continue;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        };

        if !touches_biwa_source(&event) {
            continue;
        }

        // 続けて飛んでくるイベントをまとめて捨てる。
        let deadline = Instant::now() + DEBOUNCE;
        while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            match rx.recv_timeout(remaining) {
                Ok(_) => continue,
                Err(_) => break,
            }
        }

        println!();
        if let Err(e) = build(project, target, false) {
            // 書きかけのコードは当然コンパイルが通らない。
            // 直せば次の保存でまた走るので、開発サーバは動かし続ける。
            eprintln!("error: {e:#}");
        }
    }
}

fn touches_biwa_source(event: &notify::Event) -> bool {
    if !matches!(
        event.kind,
        notify::EventKind::Create(_) | notify::EventKind::Modify(_) | notify::EventKind::Remove(_)
    ) {
        return false;
    }

    event
        .paths
        .iter()
        .any(|p| p.extension().and_then(|e| e.to_str()) == Some(biwac_base::BIWA_EXTENSION))
}

/// Vite の子プロセス。CLI が先に終わっても道連れにする。
struct ViteProcess(std::process::Child);

impl ViteProcess {
    fn spawn(project: &Project, port: Option<u16>) -> Result<Self> {
        runtime::spawn_vite(project, port).map(Self)
    }

    fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>> {
        self.0
            .try_wait()
            .context("failed to check the vite dev server")
    }
}

impl Drop for ViteProcess {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}
