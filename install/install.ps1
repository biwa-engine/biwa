# Biwa のインストーラ (Windows)。
#
#   irm https://raw.githubusercontent.com/biwa-engine/biwa/main/install/install.ps1 | iex
#
# 版を選ぶとき (stable / nightly / 0.1.0 / <commit>):
#   $env:BIWA_TOOLCHAIN_SPEC = "nightly"; irm https://raw.githubusercontent.com/biwa-engine/biwa/main/install/install.ps1 | iex
#
# やること:
#   1. GitHub Releases から、Windows 向けの CLI (`biwa-cli-<name>-windows-x86_64.exe`) を落とし、SHA256SUMS で確かめる
#   2. その CLI に `biwa toolchain install <name> --default` をさせる
#      (ツールチェインを `%USERPROFILE%\.biwa\toolchains\<name>\` に入れ、ランチャー `%USERPROFILE%\.biwa\bin\biwa.exe` を置く)
#   3. `%USERPROFILE%\.biwa\bin` を利用者の PATH に足す
#
# 環境変数:
#   BIWA_TOOLCHAIN_SPEC  入れるもの (省略すると最新の版、まだ無ければ最新の nightly)
#   BIWA_HOME            入れる場所 (既定 %USERPROFILE%\.biwa)
#   BIWA_NO_MODIFY_PATH  1 なら PATH を書き換えない
#   BIWA_RELEASES_URL    release の一覧 (試験用)
#   GITHUB_TOKEN         あれば GitHub の API に使う

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Say($message) { Write-Host "biwa-install: $message" }

$spec = $env:BIWA_TOOLCHAIN_SPEC
$biwaHome = if ($env:BIWA_HOME) { $env:BIWA_HOME } else { Join-Path $env:USERPROFILE ".biwa" }
$releasesUrl = if ($env:BIWA_RELEASES_URL) { $env:BIWA_RELEASES_URL } else { "https://api.github.com/repos/biwa-engine/biwa/releases" }
$hostId = "windows-x86_64"

if ($env:PROCESSOR_ARCHITECTURE -ne "AMD64") {
  throw "Biwa is not released for Windows $($env:PROCESSOR_ARCHITECTURE) yet"
}

$headers = @{ "User-Agent" = "biwa-install"; "Accept" = "application/vnd.github+json" }
if ($env:GITHUB_TOKEN) { $headers["Authorization"] = "Bearer $($env:GITHUB_TOKEN)" }

Say "fetching the list of releases"
$releases = @(Invoke-RestMethod -Uri "$($releasesUrl)?per_page=100" -Headers $headers) | Where-Object { -not $_.draft }

function Get-ToolchainName($release) {
  if ($release.tag_name -match '^nightly-(.+)$') { return $Matches[1] }
  if ($release.tag_name -match '^v(\d+\.\d+\.\d+)$') { return $Matches[1] }
  return $null
}

$stable = @($releases | Where-Object { $_.tag_name -match '^v\d+\.\d+\.\d+$' } |
  Sort-Object -Descending { [version]($_.tag_name.Substring(1)) })
$nightly = @($releases | Where-Object { $_.tag_name -like 'nightly-*' } | Sort-Object -Descending { $_.published_at })

$release = switch -Regex ($spec) {
  '^$' { if ($stable.Count -gt 0) { $stable[0] } elseif ($nightly.Count -gt 0) { $nightly[0] } else { $null }; break }
  '^stable$' { if ($stable.Count -gt 0) { $stable[0] } else { $null }; break }
  '^nightly$' { if ($nightly.Count -gt 0) { $nightly[0] } else { $null }; break }
  '^v?\d+\.\d+\.\d+$' { $v = $spec.TrimStart('v'); $releases | Where-Object { $_.tag_name -eq "v$v" } | Select-Object -First 1; break }
  default { $releases | Where-Object { $_.tag_name -eq "nightly-$spec" } | Select-Object -First 1 }
}
if (-not $release) {
  $what = if ($spec) { $spec } else { "the newest release" }
  throw "no released Biwa toolchain matches '$what'"
}

$name = Get-ToolchainName $release
$assetName = "biwa-cli-$name-$hostId.exe"
$asset = $release.assets | Where-Object { $_.name -eq $assetName } | Select-Object -First 1
$sumsAsset = $release.assets | Where-Object { $_.name -eq "SHA256SUMS" } | Select-Object -First 1
if (-not $asset -or -not $sumsAsset) { throw "the release $($release.tag_name) has no $assetName or SHA256SUMS" }
Say "installing the toolchain '$name' ($($release.tag_name))"

$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("biwa-install-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
  $exe = Join-Path $tmp "biwa.exe"
  Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $exe -UseBasicParsing
  $sums = (Invoke-WebRequest -Uri $sumsAsset.browser_download_url -UseBasicParsing).Content
  if ($sums -is [byte[]]) { $sums = [System.Text.Encoding]::UTF8.GetString($sums) }
  $line = ($sums -split "`n") | Where-Object { $_ -match "^\s*([0-9a-fA-F]{64})\s+\*?$([regex]::Escape($assetName))\s*$" } | Select-Object -First 1
  if (-not $line) { throw "SHA256SUMS has no entry for $assetName" }
  $want = ($line.Trim() -split '\s+')[0].ToLower()
  $got = (Get-FileHash -Algorithm SHA256 -Path $exe).Hash.ToLower()
  if ($want -ne $got) { throw "the checksum of $assetName does not match (expected $want, got $got)" }

  # 落とした CLI にツールチェインを入れさせる (圧縮ファイルの取得・確認・展開・ランチャーの配置・既定の設定)。
  New-Item -ItemType Directory -Force -Path $biwaHome | Out-Null
  $env:BIWA_HOME = $biwaHome
  & $exe toolchain install $name --default
  if ($LASTEXITCODE -ne 0) { throw "biwa toolchain install failed ($LASTEXITCODE)" }
} finally {
  Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}

$bin = Join-Path $biwaHome "bin"
if ($env:BIWA_NO_MODIFY_PATH -ne "1") {
  $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
  $parts = @(if ($userPath) { $userPath -split ';' | Where-Object { $_ } } else { @() })
  if ($parts -notcontains $bin) {
    [Environment]::SetEnvironmentVariable("Path", (@($bin) + $parts) -join ';', "User")
    Say "added $bin to your PATH (open a new terminal to use it)"
  }
}
if (($env:Path -split ';') -notcontains $bin) { $env:Path = "$bin;$env:Path" }

Say "done. Biwa is installed in $biwaHome"
