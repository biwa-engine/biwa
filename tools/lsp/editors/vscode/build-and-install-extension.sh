#!/bin/sh

# Rust バイナリをビルド, 配置
cargo build --manifest-path ../../Cargo.toml --release -p biwa_lsp_server
cp ../../target/release/biwa-lsp bin/biwa-lsp

# 拡張機能の依存関係をインストール (初回、または package.json 変更時)
npm i

# 拡張機能をビルド, インストール
npx vsce package --no-dependencies
code --install-extension biwa-lang-0.1.0.vsix
