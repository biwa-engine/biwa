```
# 1. Rust バイナリをビルド・配置 (変更のたびに必要)
cargo build --release -p biwa-lsp-server
cp target/release/biwa-lsp editors/vscode/bin/biwa-lsp

# 2. VSCode で editors/vscode/ フォルダを開く
code editors/vscode/

# 3. F5 → Extension Development Host で .biwa ファイルを開く
# アウトプットパネル "Biwa Language Server" でサーバログ確認可能

# または VSIX でインストール
cd editors/vscode && npx vsce package --no-dependencies
code --install-extension biwa-lang-0.1.0.vsix
```
