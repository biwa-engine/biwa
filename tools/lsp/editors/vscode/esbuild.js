// `vscode-languageclient` などの依存を `dist/extension.js` 1 本にバンドルする。
//
// これまで `out/extension.js` (tsc の素の出力) を `main` にしていたが、それだと
// 実行時に `require('vscode-languageclient/node')` が `node_modules/` を探しに行く。
// `.vscodeignore` が `node_modules/**` を除外しているため、`vsce package` で
// 作った .vsix には依存が入っておらず、インストール後に
// `Cannot find module 'vscode-languageclient/node'` で activate に失敗していた。
//
// `vscode` 拡張 API だけは実行時に VSCode 本体が提供するので `external` にする。
const esbuild = require('esbuild');

const production = process.argv.includes('--production');
const watch = process.argv.includes('--watch');

async function main() {
  const ctx = await esbuild.context({
    entryPoints: ['src/extension.ts'],
    bundle: true,
    format: 'cjs',
    platform: 'node',
    target: 'node18',
    outfile: 'dist/extension.js',
    external: ['vscode'],
    sourcemap: !production,
    minify: production,
    logLevel: 'info',
  });

  if (watch) {
    await ctx.watch();
  } else {
    await ctx.rebuild();
    await ctx.dispose();
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
