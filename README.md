# Biwa

**Biwaエンジン** はビジュアルノベルゲームの構築を容易にするゲームエンジンです。

ストーリー記述のシンプルさと豊かな表現力を兼ね備えるように設計された**Biwa言語**でゲームを記述します。

**Biwa Engine** is a game engine designed to facilitate the creation of visual novel games.

Games are written using the **Biwa language**, which is designed to combine simplicity in storytelling with rich expressive capabilities.

## インストール / Install

まだ版 (v0.1.0 など) は出ていないので、nightly を入れます。
No versioned release exists yet, so install a nightly.

```sh
# macOS / Linux
curl -sSf https://raw.githubusercontent.com/biwa-engine/biwa/main/install/install.sh | sh -s -- --toolchain nightly
```

```powershell
# Windows (PowerShell)
$env:BIWA_TOOLCHAIN_SPEC = "nightly"; irm https://raw.githubusercontent.com/biwa-engine/biwa/main/install/install.ps1 | iex
```

更新は `biwa update`、詳しくは [docs/toolchain.md](./docs/toolchain.md)。
Update with `biwa update`; see [docs/toolchain.md](./docs/toolchain.md) for details.

## ライセンス / LICENSE

MIT ライセンスです。 [LICENSE](./LICENSE)を参照してください。
ただし、より下層のディレクトリにライセンスが示されている場合、そのライセンスに従ってください。

MIT Licensed. See [LICENSE](./LICENSE).
However, if a license is specified in a subdirectory, please follow it.
