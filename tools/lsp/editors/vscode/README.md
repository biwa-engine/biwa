# Biwa Language Server (VSCode Extension)

```
# 1. Rust バイナリをビルド・配置 (変更のたびに必要)
cargo build --release -p biwa_lsp_server
cp target/release/biwa-lsp editors/vscode/bin/biwa-lsp

# 2. 拡張機能の依存関係をインストール (初回、または package.json 変更時)
cd editors/vscode && npm install

# 3. VSCode で editors/vscode/ フォルダを開く
code .

# 4. F5 → Extension Development Host で .biwa ファイルを開く
# アウトプットパネル "Biwa Language Server" でサーバログ確認可能

# または VSIX でインストール
npx vsce package --no-dependencies
code --install-extension biwa-lang-0.1.0.vsix
```

拡張機能本体 (`src/extension.ts`) は esbuild で `dist/extension.js` に
`vscode-languageclient` ごとバンドルしている (`npm run compile` / `esbuild.js`)。
`node_modules` はパッケージに含めない (`.vscodeignore`) ので、バンドルせずに
`out/` を `main` にすると `vsce package` 後に
`Cannot find module 'vscode-languageclient/node'` で activate に失敗する。

`@vscode/vsce` は 3.x 以降が Node 20+ を要求する (4.x は Node 22+)。
このリポジトリは Node 18 でも動くよう `@vscode/vsce@^2.32.0` を使い、
その依存が引き込む Node20+ 専用パッケージ (`@azure/msal-node`, `cheerio`)
を `package.json` の `overrides` で Node18 互換バージョンに固定してある。
Node 20+ が使える環境ではこの overrides は不要 (害もない)。

## License

MIT-Licensed. See [./LICENSE].
