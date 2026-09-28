# Biwa Package Hub

Biwa のライブラリパッケージをグローバルに格納するハブ(レジストリ)。

コンパイラや `biwa` CLI, LSP などの各種ツールはハブから必要な依存パッケージを取得する。

Biwa言語では、パッケージにはグローバルに一意な名前を持つが、
ハブに一意な名前で登録されることでこれが達成される。

## 格納するデータ

### Phase 1 暫定実装

パッケージのメタデータのみ管理し、ソース自体はGitHubなどの公開Gitリポジトリに委ねる。
各バージョンはGitコミットハッシュで不変性を担保する。
ソースを保持しないため、Gitリポジトリ側が非公開にされてしまいビルドできなくなるケースがありうるが、
現時点ではすべてのBiwaソースをBiwa開発チームで維持管理するのは現実的ではないため妥協する。
バージョンごとに `biwa-package.json` にあるメタデータは記録される。

- `/v1/packages/<package-name>/`:

```json
{
  "id": "<UUID>",
  "name": "<パッケージ名>",
  "repository": "<gitリポジトリのURL(GitHubなど)>",
  // Phase 1 はユーザアカウント/認証が無いため常に null。
  // 認証実装後、公開者に紐づくようになる。
  "owner": {
    "id": "<パッケージを所有するBiwa HubアカウントのUUID>",
    "visible_id": "[a-zA-Z0-9-_]+"
  } | null,
  "latest": "<major>.<minor>.<patch>",
  "created_at": "<最初に公開された時刻(RFC3339)>",
  "updated_at": "<最後に更新された時刻(RFC3339)>"
}
```

uuid からも引ける (依存解決でコンパイラ/CLI/LSP が `versions` の `dependencies` に載っている
UUID からパッケージ名を逆引きする必要があるため):

- `/v1/packages/?uuid=<package-uuid>`: 上と同じレスポンス形。

- `/v1/packages/<package-name>/versions/`:

```json
[
  {
    "version": "<major>.<minor>.<patch>",
    "description": "<説明>",
    "commit": "<コミットハッシュ>",

    // 直接の依存。コンパイラなどはこれを再帰的に見て回ることで不整合がないパッケージ依存ツリーを解決する。
    // `name` は取得先ディレクトリ (`deps/<name>/`) を決めるために必要なので同梱する
    // (依存の一意性そのものは `id` (UUID) で判定する — 万一名前が改名されても壊れない)。
    "dependencies": [
      {
        "id": "<UUID>",
        "name": "<パッケージ名>",
        "version": "<major>.<minor>.<patch>"
      }
    ],
    "created_at": "<公開された時刻(RFC3339, 公開は取り消すことはできない)>"
  }
]
```

#### パッケージの登録・公開

認証は Phase 2 (ユーザアカウント実装後) で必須になる想定だが、
それまでは誰でも登録・公開できる (`owner` は常に `null` になる)。

- `POST /v1/packages/`: 新規パッケージを登録する。

```json
// request
{
  "name": "<パッケージ名>",
  "repository": "<gitリポジトリのURL>"
}
```

登録直後はバージョンが 1 つも無いので `latest` を持たない。
レスポンスは `/v1/packages/<package-name>/` と同じ形 (`latest` は `null`)。
同名パッケージが既にあれば衝突エラーになる。

- `POST /v1/packages/<package-name>/versions/`: 新しいバージョンを公開する。

```json
// request
{
  "version": "<major>.<minor>.<patch>",
  "description": "<説明>",
  "commit": "<コミットハッシュ>",
  // 依存は id (UUID) で指定する。公開する側 (`biwa publish` 相当) が
  // 事前に `GET /v1/packages/<name>/` で名前から id を引いておく。
  "dependencies": [
    { "id": "<UUID>", "version": "<major>.<minor>.<patch>" }
  ]
}
```

公開は取り消せない (バージョンの不変性を担保するため)。
同じパッケージに同じ `version` を 2 回公開しようとするとエラーになる。
レスポンスは登録されたバージョン 1 件分 (`GET .../versions/` の要素と同じ形、`name` 付き)。

#### ユーザアカウント

Biwa Package Hub は独自にユーザアカウントを管理する。

当面GitHubアカウントでのみサインインできるものとする。

アカウントは(内部UUIDのほか)一意な表示ID(visible_id, `[a-zA-Z0-9-_]`のみ使用可能)を持ち、
サインアップ時に設定可能であるが、デフォルトではGitHubの表示IDと同じものが自動で設定される。

- `/v1/users/<user-visible-id>/`:

```json
{
  "id": "<UUID>",
  "visible_id": "[a-zA-Z0-9-_]+",
  "created_at": "<登録された時刻(RFC3339)>",
  "updated_at": "<最後に更新された時刻(RFC3339)>"
}
```

- `/v1/users/signin/`: サインインできる。

##### API トークン

アカウントに対してAPI トークンを発行することが出来、

Biwa CLI でログイン(`biwa login`)し、パッケージ公開時(`biwa publish`)の認証に使われる。
Biwa CLI は人間が手動で使うこともできれば、CI/CDに組み込んで使うこともできる。

- `/v1/users/token/`: APIトークンを発行できる。

### Phase 2

将来的にソースもすべてハブで管理されるようになる可能性がある。

## hub に関連するコンポーネント、サービス

- `server`
  hub の Web API そのもの。

- `client`
  hub の API を使用するクライアントのRustライブラリ。コンパイラやBiwa CLI, LSPからライブラリとして使われる。
  hub API の URL はビルド時に環境変数から取り込まれて決定される。

- `website`
  hub の Web サイト。ユーザはここでパッケージを検索できたり、パッケージ情報を閲覧できたりする。
  ユーザアカウントのサインアップ/サインイン/APIトークン発行もここから行える。
  裏では`server` の Web APIが呼び出されている。

## 今回実装する範囲

- `server`: レイヤードアーキテクチャ (`biwa_hub_domain` / `biwa_hub_infrastructure` /
  `biwa_hub_usecase` / `biwa_hub_presentation`) で、パッケージ登録・バージョン公開・
  名前 / uuid での取得の API を実装する。永続化は Postgres (sqlx)。
  ユーザアカウント・認証・API トークンは実装しない (`owner` は常に `null`)。
- `client`: 上記 API を叩く同期 (blocking) な Rust クライアント。
- コンパイラ / CLI / LSP: 依存が `.biwa_build/deps/<name>/` に無いとき、
  `client` 経由でハブから解決して git 越しに取得する (下記)。
- `website`, `biwa publish` (CLI コマンド) は対象外。パッケージの登録・公開は
  `client` を直接叩くこと (または `server` への直接リクエスト) で行える。

### 自動フェッチのアルゴリズム

コンパイラなど (`biwac_driver::DepGraph::discover`) が依存パッケージを
`packages_dir/<name>/` に見つけられなかったとき:

1. `client` で `GET /v1/packages/<name>/` を呼び、パッケージの `repository` と
   バージョン一覧の取得先を得る。
2. `GET /v1/packages/<name>/versions/` から、依存側が要求する
   `min..max` の範囲を満たす最新バージョンを選ぶ。
3. 選んだバージョンの `commit` を使い、`repository` を git 越しに
   `packages_dir/<name>/` へ取得する (`git init` → `git fetch <repo> <commit>` →
   `git checkout FETCH_HEAD` を想定。shallow フェッチ)。
4. 選んだバージョンの `dependencies` (id + name + version) を、
   まだ `visited` でなければキューに積んで 1〜4 を繰り返す
   (id は将来の改名耐性のため、実際の取得は name ベースで行う)。

`DepGraph::discover` 自体は既に BFS で推移閉包を辿る形になっているため、
「ディレクトリが無ければ即エラー」だった箇所を「無ければ上記を試みる」に差し替える形で
組み込む。
