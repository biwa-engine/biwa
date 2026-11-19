//! ツールチェインを入れる・ランチャーを入れ替える。
//!
//! 圧縮ファイル (`biwa-<name>-<host>.{tar.gz,zip}`) の中身は `~/.biwa` と同じ形である:
//!
//! ```text
//! bin/biwa[.exe]                       ← ランチャー (ツールチェインの biwa の写し)
//! toolchains/<name>/bin/biwa[.exe]
//! toolchains/<name>/runtime/<id>/...
//! ```
//!
//! いったん `<home>/tmp/` に展開し、`toolchains/<name>` を rename で置く。途中で止まっても、
//! 中途半端なツールチェインが `toolchains/` に並ぶことはない。

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use super::home::{EXE, Home};
use super::releases::{self, Release};

/// 展開した圧縮ファイルから、入っているツールチェインの名前と場所を取る。
struct Unpacked {
    /// 展開先 (`<home>/tmp/…`)。用が済んだら消す。
    dir: PathBuf,
    name: String,
}

impl Unpacked {
    fn toolchain(&self) -> PathBuf {
        self.dir.join("toolchains").join(&self.name)
    }
}

impl Drop for Unpacked {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 入れた結果。
pub enum Installed {
    New(String),
    /// 同じ名前のものが既にあった (`force` でなければ何もしない)。
    AlreadyThere(String),
}

impl Installed {
    pub fn name(&self) -> &str {
        match self {
            Self::New(n) | Self::AlreadyThere(n) => n,
        }
    }
}

/// release からこのホスト向けのツールチェインを入れる。
pub fn from_release(
    home: &Home,
    client: &reqwest::blocking::Client,
    release: &Release,
    force: bool,
) -> Result<Installed> {
    let name = release
        .toolchain_name()
        .context("not a toolchain release")?;
    if home.is_installed(&name) && !force {
        return Ok(Installed::AlreadyThere(name));
    }
    let host = releases::host()?;
    let asset = release.asset(&releases::archive_asset(&name, host))?;
    let sums = checksums(client, release)?;

    let archive = download(
        home,
        client,
        &asset.browser_download_url,
        &asset.name,
        asset.size,
    )?;
    let result = verify(&archive, &asset.name, &sums)
        .and_then(|()| from_archive(home, &archive, Some(&name), force));
    discard(&archive);
    result
}

/// 手元の圧縮ファイルから入れる。`expected` があれば、中身の名前がそれと同じか確かめる。
pub fn from_archive(
    home: &Home,
    archive: &Path,
    expected: Option<&str>,
    force: bool,
) -> Result<Installed> {
    let unpacked = unpack(home, archive)?;
    if let Some(expected) = expected
        && unpacked.name != expected
    {
        bail!(
            "{} contains the toolchain `{}`, not `{expected}`",
            archive.display(),
            unpacked.name
        );
    }
    let name = unpacked.name.clone();
    if !unpacked.toolchain().join("bin").join(EXE).is_file() {
        bail!(
            "{} is not a Biwa toolchain for this host (toolchains/{name}/bin/{EXE} is missing)",
            archive.display()
        );
    }
    if home.is_installed(&name) && !force {
        return Ok(Installed::AlreadyThere(name));
    }
    place(home, &unpacked.toolchain(), &name)?;
    Ok(Installed::New(name))
}

/// `src` (展開したツールチェイン) を `toolchains/<name>` に置く。あれば入れ替える。
fn place(home: &Home, src: &Path, name: &str) -> Result<()> {
    let dst = home.toolchain_dir(name);
    std::fs::create_dir_all(home.toolchains_dir())?;
    if dst.exists() {
        // 入れ替えは「古いものを退かす → 新しいものを置く → 古いものを消す」。
        let old = unique_tmp(home, &format!("old-{name}"))?;
        std::fs::rename(&dst, &old)
            .with_context(|| format!("failed to move {} away (is it in use?)", dst.display()))?;
        if let Err(e) = std::fs::rename(src, &dst) {
            let _ = std::fs::rename(&old, &dst);
            return Err(e).with_context(|| format!("failed to place {}", dst.display()));
        }
        let _ = std::fs::remove_dir_all(&old);
    } else {
        std::fs::rename(src, &dst).with_context(|| format!("failed to place {}", dst.display()))?;
    }
    Ok(())
}

/// 圧縮ファイルを `<home>/tmp/` に展開する。
fn unpack(home: &Home, archive: &Path) -> Result<Unpacked> {
    let dir = unique_tmp(home, "unpack")?;
    std::fs::create_dir_all(&dir)?;
    // ここから先で失敗しても、展開先は Drop で消える。
    let mut unpacked = Unpacked {
        dir,
        name: String::new(),
    };

    let file = std::fs::File::open(archive)
        .with_context(|| format!("failed to open {}", archive.display()))?;
    let file_name = archive
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    if file_name.ends_with(".zip") {
        let mut zip = zip::ZipArchive::new(file)
            .with_context(|| format!("{} is not a zip file", archive.display()))?;
        // `extract` は外へ出るパス (`../`) を弾く。Unix では実行ビットも戻す。
        zip.extract(&unpacked.dir)
            .with_context(|| format!("failed to extract {}", archive.display()))?;
    } else if file_name.ends_with(".tar.gz") || file_name.ends_with(".tgz") {
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
        tar.set_preserve_permissions(true);
        // `unpack` は外へ出るパスを弾く。
        tar.unpack(&unpacked.dir)
            .with_context(|| format!("failed to extract {}", archive.display()))?;
    } else {
        bail!("{} is neither a .tar.gz nor a .zip file", archive.display());
    }

    let names: Vec<String> = std::fs::read_dir(unpacked.dir.join("toolchains"))
        .with_context(|| format!("{} has no `toolchains/` directory", archive.display()))?
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    let [name] = names.as_slice() else {
        bail!(
            "{} must contain exactly one toolchain, found: {names:?}",
            archive.display()
        );
    };
    super::name::validate(name)?;
    unpacked.name = name.clone();
    Ok(unpacked)
}

/// `<home>/tmp/<prefix>-<pid>-<n>` (まだ無い名前)。
fn unique_tmp(home: &Home, prefix: &str) -> Result<PathBuf> {
    let tmp = home.tmp_dir();
    std::fs::create_dir_all(&tmp).with_context(|| format!("failed to create {}", tmp.display()))?;
    for n in 0.. {
        let p = tmp.join(format!("{prefix}-{}-{n}", std::process::id()));
        if !p.exists() {
            return Ok(p);
        }
    }
    unreachable!()
}

/// `SHA256SUMS` を読む (`<hex>  <file>` の行)。
fn checksums(
    client: &reqwest::blocking::Client,
    release: &Release,
) -> Result<Vec<(String, String)>> {
    let asset = release.asset(releases::CHECKSUMS)?;
    let text = client
        .get(&asset.browser_download_url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .and_then(reqwest::blocking::Response::text)
        .with_context(|| format!("failed to download {}", asset.browser_download_url))?;
    Ok(text
        .lines()
        .filter_map(|l| {
            let (hash, file) = l.split_once(char::is_whitespace)?;
            Some((
                hash.to_ascii_lowercase(),
                file.trim().trim_start_matches('*').to_string(),
            ))
        })
        .collect())
}

fn verify(path: &Path, asset_name: &str, sums: &[(String, String)]) -> Result<()> {
    let Some((want, _)) = sums.iter().find(|(_, f)| f == asset_name) else {
        bail!("`{}` has no checksum for {asset_name}", releases::CHECKSUMS);
    };
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let got: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if &got != want {
        bail!(
            "the checksum of {asset_name} does not match (expected {want}, got {got}); the download may be broken"
        );
    }
    Ok(())
}

/// `url` を `<home>/tmp/download-…/<label>` に落とす (拡張子で形式を見分けるので、名前はそのまま)。
/// 用が済んだら [`discard`] で消す。
fn download(
    home: &Home,
    client: &reqwest::blocking::Client,
    url: &str,
    label: &str,
    size: u64,
) -> Result<PathBuf> {
    let dir = unique_tmp(home, "download")?;
    std::fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    let path = dir.join(label);
    if size > 0 {
        eprintln!("downloading {label} ({:.1} MB)", size as f64 / 1_048_576.0);
    } else {
        eprintln!("downloading {label}");
    }
    let mut response = client
        .get(url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .with_context(|| format!("failed to download {url}"))?;
    let mut file = std::fs::File::create(&path)
        .with_context(|| format!("failed to create {}", path.display()))?;
    let copied = response
        .copy_to(&mut file)
        .map_err(anyhow::Error::from)
        .and_then(|_| file.flush().map_err(anyhow::Error::from));
    if let Err(e) = copied {
        discard(&path);
        return Err(e.context(format!("failed to download {url}")));
    }
    Ok(path)
}

/// [`download`] で落としたものを、置いたディレクトリごと消す。
fn discard(path: &Path) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// release の CLI (`biwa-cli-<name>-<host>`) を落とし、確かめて、ランチャーにする。
pub fn launcher_from_release(
    home: &Home,
    client: &reqwest::blocking::Client,
    release: &Release,
) -> Result<()> {
    let name = release
        .toolchain_name()
        .context("not a toolchain release")?;
    let host = releases::host()?;
    let asset = release.asset(&releases::cli_asset(&name, host))?;
    let sums = checksums(client, release)?;
    let path = download(
        home,
        client,
        &asset.browser_download_url,
        &asset.name,
        asset.size,
    )?;
    let result = verify(&path, &asset.name, &sums).and_then(|()| replace_launcher(home, &path));
    discard(&path);
    result
}

/// ランチャー (`<home>/bin/biwa`) を `src` の写しに入れ替える。実行中のランチャーでもよい。
pub fn replace_launcher(home: &Home, src: &Path) -> Result<()> {
    let dst = home.launcher();
    let bin = dst.parent().expect("the launcher is in bin/");
    std::fs::create_dir_all(bin).with_context(|| format!("failed to create {}", bin.display()))?;

    // 同じディレクトリに写してから rename で置く (途中で止まっても壊れたランチャーが残らない)。
    let staged = bin.join(format!("{EXE}.new"));
    std::fs::copy(src, &staged)
        .with_context(|| format!("failed to copy {} to {}", src.display(), staged.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
    }

    // Windows は実行中の exe を上書きも削除もできないが、改名はできる。退かしておき、次に起動したときに消す
    // (`cleanup_old_launcher`)。
    if cfg!(windows) && dst.exists() {
        let old = old_launcher(home);
        let _ = std::fs::remove_file(&old);
        std::fs::rename(&dst, &old)
            .with_context(|| format!("failed to move {} away", dst.display()))?;
    }
    std::fs::rename(&staged, &dst).with_context(|| format!("failed to place {}", dst.display()))?;
    Ok(())
}

fn old_launcher(home: &Home) -> PathBuf {
    home.launcher().with_file_name(format!("{EXE}.old"))
}

/// 前に入れ替えたときに退かしたランチャーを消す (Windows)。消せなくても構わない。
pub fn cleanup_old_launcher(home: &Home) {
    let old = old_launcher(home);
    if old.exists() {
        let _ = std::fs::remove_file(old);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifies_checksums() {
        let dir = std::env::temp_dir().join(format!("biwa-verify-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.tar.gz");
        std::fs::write(&file, b"hello").unwrap();
        let good = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824".to_string();
        assert!(verify(&file, "a.tar.gz", &[(good.clone(), "a.tar.gz".into())]).is_ok());
        assert!(verify(&file, "a.tar.gz", &[("00".into(), "a.tar.gz".into())]).is_err());
        assert!(verify(&file, "a.tar.gz", &[(good, "b.tar.gz".into())]).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
