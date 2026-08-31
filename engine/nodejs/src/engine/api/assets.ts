/**
 * アセットの置き場所と、`.biwa` が書くパスの解決。
 *
 * 規約は「アセットはパッケージ直下の `assets/` に置き、
 * `.biwa` からは `assets/` を基準とした相対パスで参照する」である。
 * `biwa dev` はその `assets/` をエンジンの公開ディレクトリへ繋ぐので、
 * エンジンから見ると常に `<base>assets/<path>` に居る。
 *
 * 画像だけでなく音や動画の syscall が増えても同じ規約に従わせるため、
 * 解決はここ 1 箇所にまとめてある。
 */

/** `assets/` が公開される位置 (`biwa dev` が張るリンクの名前と対で決まる)。 */
const ASSETS_ROOT = "assets";

/**
 * `.biwa` が書いたパスをブラウザから引ける URL に直す。
 *
 * パスは `assets/` を基準とした相対パスなので、`assets/` 自体は含まない。
 * 規約に反するパスは投げる。呼ぶ側が握り潰すか止まるかを決める。
 */
export function resolveAssetUrl(path: string): string {
  const segments: string[] = [];

  // 先頭の `/` は「assets ルートから」の意味に読み替えて落とす。
  for (const segment of path.replace(/^\/+/, "").split("/")) {
    if (segment === "" || segment === ".") {
      continue;
    }
    if (segment === "..") {
      // 抜けられると `assets/` に置くという規約自体が意味を失う。
      // 将来はコンパイラが静的に弾く場所でもある。
      throw new Error(
        `[biwa] asset path must stay inside \`assets/\`: "${path}"`,
      );
    }
    // 空白や日本語のファイル名をそのまま書けるようにする。
    segments.push(encodeURIComponent(segment));
  }

  if (segments.length === 0) {
    throw new Error("[biwa] asset path is empty");
  }

  // BASE_URL は末尾に `/` を持つ。ルート以外に配信されても壊れないように通す。
  return `${import.meta.env.BASE_URL}${ASSETS_ROOT}/${segments.join("/")}`;
}
