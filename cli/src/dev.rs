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
}

pub fn run(args: DevArgs) -> Result<()> {
    let start_dir = args
        .package_path
        .clone()
        .unwrap_or_else(|| PathBuf::from("."));
    let project = Project::discover(&start_dir)?;

    println!("Package: {} ({})", project.name, project.root.display());

    runtime::ensure_std(&project)?;
    runtime::ensure_engine(&project)?;

    // 初回は通らないと開発サーバを立てる意味がないので、失敗したらそこで止める。
    build(&project, args.rebuild)?;

    let mut vite = ViteProcess::spawn(&project, args.port)?;

    watch_loop(&project, &mut vite)
}

/// biwac を呼んで、生成物をエンジン側へ配る。
fn build(project: &Project, force_rebuild: bool) -> Result<()> {
    compile(project, force_rebuild)?;
    runtime::sync_generated(project)
}

/// コンパイラ本体の呼び出し。
///
/// biwac はライブラリとして呼ぶ (`biwac` コマンドは経由しない)。
/// まだ未実装のエラー表示経路で panic することがあるので、
/// 開発サーバごと落ちないようにここで受け止める。
fn compile(project: &Project, force_rebuild: bool) -> Result<()> {
    let root = project.root.clone();

    let result = std::panic::catch_unwind(move || {
        biwac_driver::compile(root, biwac_driver::BuildOptions { force_rebuild })
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
fn watch_loop(project: &Project, vite: &mut ViteProcess) -> Result<()> {
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
        if let Err(e) = build(project, false) {
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

    event.paths.iter().any(|p| {
        p.extension().and_then(|e| e.to_str()) == Some(biwac_base::BIWA_EXTENSION)
    })
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
