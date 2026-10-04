//! `biwa publish`: パッケージを Biwa Package Hub に公開する。
//!
//! やることは順に:
//!   1. `biwa-package.json` を読む (名前・バージョン・説明・依存)。
//!   2. コミットを決める (`--commit` 省略時は HEAD の確認)。
//!   3. 直接依存それぞれを、ローカルに取得済みの実体から
//!      「正確なバージョン」と「hub 上の id」に解決する。
//!   4. (初回のみ) git remote から公開先リポジトリを選ばせ (自由記述は認めない)、
//!      hub にパッケージを登録する。`--update` のときはこの手順自体が無い —
//!      repository はパッケージに 1 つしか持てず hub 側に変更 API も無いので、
//!      既に登録されているものをそのまま使う。
//!   5. バージョンを公開する。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Args;

use biwa_hub_client::{HubClient, HubClientError, dto};

use crate::project::Project;

#[derive(Debug, Args)]
pub struct PublishArgs {
    /// 対象の biwa パッケージ (省略時はカレントディレクトリから探す)
    #[arg(short, long, value_name = "DIR")]
    pub package_path: Option<PathBuf>,

    /// 既に公開済みのパッケージへ、新しいバージョンを追加する。
    ///
    /// 付けなければ新規登録 (`POST /v1/packages/`) から始める。
    #[arg(long)]
    pub update: bool,

    /// 公開するコミットハッシュ。省略すると `HEAD` を使ってよいか確認される。
    #[arg(long)]
    pub commit: Option<String>,
}

pub fn run(args: PublishArgs) -> Result<()> {
    let start_dir = args
        .package_path
        .clone()
        .unwrap_or_else(|| PathBuf::from("."));
    let project = Project::discover(&start_dir)?;

    // `Project` (project.rs) は名前しか見ないので、フルのメタデータを読み直す。
    let metadata = biwac_metadata_loader::try_load_package_metadata(project.root.clone())
        .map_err(|e| anyhow::anyhow!("{e:?}"))
        .with_context(|| format!("failed to load {}", biwac_base::METADATA_FILE_NAME))?;

    let name = metadata.metadata.name.value().to_string();
    let version = metadata.metadata.version.to_string();
    println!("Package: {name} v{version} ({})", project.root.display());

    let hub = HubClient::new().context(
        "hub is not configured (`BIWA_HUB_URL` was not set when this `biwa` binary was built)",
    )?;

    let commit = resolve_commit(&project.root, args.commit.as_deref())?;

    // 依存解決は登録より前に済ませる。ここで失敗したときに
    // 「パッケージだけ登録されてバージョンは無い」半端な状態を残さないため。
    let dependencies = resolve_dependencies(&project, &metadata, &hub)?;

    if args.update {
        // repository はパッケージに 1 つしか持てず、hub にも変更する API が無い
        // (バージョンごとに変えられるようにする方針変更が入るまでは)。
        // なので remote は選ばせず、登録済みの値をそのまま使う。
        hub.get_package_by_name(&name).map_err(|e| match e {
            HubClientError::NotFound => anyhow::anyhow!(
                "`{name}` is not registered on the hub yet; run `biwa publish` without `--update` first"
            ),
            other => anyhow::anyhow!(other),
        })?;
    } else {
        let repo_url = select_git_remote_url(&project.root)?;
        hub.register_package(&dto::RegisterPackageRequest {
            name: name.clone(),
            repository: repo_url.clone(),
        })
        .map_err(|e| match e {
            HubClientError::Conflict(_) => anyhow::anyhow!(
                "`{name}` is already registered on the hub; use `--update` to publish a new version"
            ),
            other => anyhow::anyhow!(other),
        })?;
        println!("Registered `{name}` -> {repo_url}");
    }

    let published = hub
        .publish_version(
            &name,
            &dto::PublishVersionRequest {
                version: version.clone(),
                description: metadata.metadata.description.clone(),
                commit,
                dependencies,
            },
        )
        .map_err(|e| match e {
            HubClientError::Conflict(_) => anyhow::anyhow!(
                "version `{version}` of `{name}` is already published (versions are immutable)"
            ),
            other => anyhow::anyhow!(other),
        })?;

    println!(
        "Published {name} v{} @ {}",
        published.version, published.commit
    );

    Ok(())
}

/// 直接依存それぞれを、ローカルに取得済みの実体から「正確なバージョン」と
/// 「hub 上の id」に解決する。
///
/// `biwa-package.json` の依存はバージョン範囲 (`min..max`) でしか書かれておらず、
/// hub には (再現性のため) 正確な 1 バージョンを pin して送る必要がある。
/// 「実際にこのパッケージがビルド時に使ったバージョン」を正とするため、
/// `.biwa_build/deps/<name>/` に取得済みの実体を読む
/// (`biwa dev` などで一度ビルドしておく必要がある)。
fn resolve_dependencies(
    project: &Project,
    metadata: &biwac_base::MetadataHolder,
    hub: &HubClient,
) -> Result<Vec<dto::PublishVersionDependencyRequest>> {
    let deps_dir = project.dependencies_dir();
    let mut dependencies = Vec::with_capacity(metadata.metadata.dependencies.len());

    for dep in &metadata.metadata.dependencies {
        let dep_root = deps_dir.join(dep.name.value());
        let dep_meta = biwac_metadata_loader::try_load_package_metadata(dep_root.clone())
            .map_err(|e| anyhow::anyhow!("{e:?}"))
            .with_context(|| {
                format!(
                    "dependency `{}` is not available at {} (build the package first, \
                     e.g. `biwa dev`, so its exact version can be pinned)",
                    dep.name,
                    dep_root.display()
                )
            })?;

        if !dep_meta
            .metadata
            .version
            .in_range(&dep.min_version, dep.max_version.as_ref())
        {
            bail!(
                "locally available `{}` v{} does not satisfy the declared range {}..{}",
                dep.name,
                dep_meta.metadata.version,
                dep.min_version,
                dep.max_version
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "*".to_string())
            );
        }

        let hub_pkg = hub
            .get_package_by_name(dep.name.value())
            .map_err(|e| match e {
                HubClientError::NotFound => anyhow::anyhow!(
                    "dependency `{}` is not registered on the hub yet; publish it first",
                    dep.name
                ),
                other => anyhow::anyhow!(other),
            })?;

        dependencies.push(dto::PublishVersionDependencyRequest {
            id: hub_pkg.id,
            version: dep_meta.metadata.version.to_string(),
        });
    }

    Ok(dependencies)
}

/// git remote から公開先を選ばせる。自由記述は認めない
/// (登録されていない URL を公開できてしまうと、なりすましの入口になるため)。
fn select_git_remote_url(repo_dir: &Path) -> Result<String> {
    let candidates = list_remote_candidates(repo_dir)?;

    match candidates.len() {
        0 => bail!(
            "no git remote found in `{}`; add one (e.g. `git remote add origin <url>`) before publishing",
            repo_dir.display()
        ),
        1 => {
            let (remote_name, url) = &candidates[0];
            if confirm(
                &format!("Using git remote `{remote_name}`: {url}. Proceed?"),
                true,
            )? {
                Ok(url.clone())
            } else {
                bail!("aborted");
            }
        }
        _ => {
            println!("Multiple git remotes found:");
            let items: Vec<String> = candidates
                .iter()
                .map(|(remote_name, url)| format!("{remote_name} -> {url}"))
                .collect();
            let idx = select_from_list("Select a repository to publish", &items)?;
            Ok(candidates[idx].1.clone())
        }
    }
}

/// `(remote 名, https 化した URL)` の一覧。`git remote` があるディレクトリでなければエラー。
fn list_remote_candidates(repo_dir: &Path) -> Result<Vec<(String, String)>> {
    run_git(repo_dir, &["rev-parse", "--is-inside-work-tree"])
        .with_context(|| format!("`{}` is not inside a git repository", repo_dir.display()))?;

    let names = run_git(repo_dir, &["remote"])?;
    names
        .lines()
        .filter(|l| !l.is_empty())
        .map(|remote_name| {
            let url = run_git(repo_dir, &["remote", "get-url", remote_name])?;
            Ok((
                remote_name.to_string(),
                normalize_repository_url(url.trim()),
            ))
        })
        .collect()
}

/// SSH 形式の git URL を https に変換する。
///
/// GitHub 等に SSH 鍵を通していないクライアントの方が多いと想定されるので、
/// hub には基本 https を渡す (hub 側も鍵を扱わずに済む)。
///
/// - `git@host:owner/repo.git` (scp 風) → `https://host/owner/repo.git`
/// - `ssh://[user@]host/owner/repo.git` → `https://host/owner/repo.git`
/// - それ以外 (既に https/http/git 等) はそのまま。
fn normalize_repository_url(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("git@")
        && let Some((host, path)) = rest.split_once(':')
    {
        return format!("https://{host}/{path}");
    }
    if let Some(rest) = url.strip_prefix("ssh://") {
        let rest = rest.split_once('@').map_or(rest, |(_, r)| r);
        return format!("https://{rest}");
    }
    url.to_string()
}

/// `--commit` を解決する。省略されたら `HEAD` を使ってよいか尋ねる。
/// どちらの場合も、実在するコミットであることを確認しつつ完全なハッシュに正規化する。
fn resolve_commit(repo_dir: &Path, requested: Option<&str>) -> Result<String> {
    let rev = match requested {
        Some(commit) => commit.to_string(),
        None => {
            let head = run_git(repo_dir, &["rev-parse", "HEAD"])?;
            if !confirm(&format!("No --commit given. Use HEAD ({head})?"), true)? {
                bail!("aborted; re-run with `--commit <hash>`");
            }
            head
        }
    };

    run_git(
        repo_dir,
        &["rev-parse", "--verify", &format!("{rev}^{{commit}}")],
    )
    .with_context(|| format!("`{rev}` is not a known commit in this repository"))
}

fn run_git(dir: &Path, args: &[&str]) -> Result<String> {
    let output = std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .context("failed to run `git`; is it installed?")?;

    if !output.status.success() {
        bail!(
            "`git {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn read_line(prompt: &str) -> Result<String> {
    use std::io::Write;
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

fn confirm(prompt: &str, default_yes: bool) -> Result<bool> {
    let hint = if default_yes { "[Y/n]" } else { "[y/N]" };
    loop {
        let answer = read_line(&format!("{prompt} {hint} "))?;
        match answer.to_lowercase().as_str() {
            "" => return Ok(default_yes),
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => println!("please answer y or n"),
        }
    }
}

fn select_from_list(prompt: &str, items: &[String]) -> Result<usize> {
    for (i, item) in items.iter().enumerate() {
        println!("  {}) {item}", i + 1);
    }
    loop {
        let answer = read_line(&format!("{prompt} [1-{}]: ", items.len()))?;
        if let Ok(n) = answer.parse::<usize>()
            && (1..=items.len()).contains(&n)
        {
            return Ok(n - 1);
        }
        println!("please enter a number between 1 and {}", items.len());
    }
}
