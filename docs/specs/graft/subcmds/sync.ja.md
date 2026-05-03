# graft sync — 動作モードと upstream manifest

## 1. 動作モード

### 1.1 sync(デフォルト)

```
graft sync [-m <manifest>] [--dry-run]
```

ファイルをローカルに書き込む。commit / PR は行わない。
commit・PR のワークフローは skill 経由で Claude に委ねる。

1. マニフェストをパースしバリデーション
2. upstream ファイルを取得(`GITHUB_TOKEN` 環境変数を使用)
3. 各ルールの戦略を適用

`--dry-run` を付けるとファイルへの書き込みを行わず diff のみ出力する。

### 1.2 validate (`--validate`)

```
graft sync --validate [-m <manifest>]
```

ファイルの取得や変更を行わずにマニフェストの正しさを検証。
成功で exit 0、エラーで exit 1。ルールごとのステータスを出力。

### 1.3 CI check (`--ci-check`)

```
graft sync --ci-check [-m <manifest>]
```

drift detection: 現在のローカル状態を同期後の期待状態と比較。
乖離がなければ exit 0、乖離があれば exit 1。バリデーションを内包。

ファイルは一切変更しない。

**GitHub Actions 環境での追加動作** (`GITHUB_ACTIONS=true` のとき):

1. drift が検出されたファイルごとに workflow command でアノテーションを出力する:
   ```
   ::error file={path},title=graft drift::{path} is out of sync with upstream
   ```
2. 実行が PR に紐づいている場合 (`GITHUB_REF` が `refs/pull/*/merge` のとき)、
   `gh pr comment` でドリフトサマリーをコメントとして投稿する:
   ```
   gh pr comment $PR_NUMBER --body "..."
   ```
   コメント本文には drift したファイルの一覧と各ファイルの unified diff を含める。
   PR 番号は `GITHUB_REF` から `refs/pull/<number>/merge` の形式で抽出する。

GHA 環境以外ではアノテーション・PR コメントの処理はスキップする。

### 1.4 patch-refresh (`--patch-refresh`)

```
graft sync --patch-refresh [-m <manifest>]
```

`patch` 戦略のルールに対して upstream の最新内容とローカルファイルの diff を取り、
patch ファイルを自動生成(上書き)する。patch ファイルを手書きするコストを排除する。

パイプライン:

1. マニフェストをパースしバリデーション
2. `strategy: patch` のルールのみ対象
3. upstream ファイルを取得
4. ローカルファイルを読み取り(存在しない場合は空として扱う)
5. `preserve_markers: true` の場合、upstream とローカルの両方からマーカーブロックを除去してから `diff -u` を実行する。それ以外の場合は素の `diff -u <upstream> <local>` を実行する
6. 差分がなければ `Unchanged`(patch ファイルは更新しない)
7. 解決済みパス(省略時は慣例パス)に unified diff を書き込み
8. 親ディレクトリが存在しない場合は作成

ローカルファイルが存在しない場合: `diff -u <upstream> /dev/null` に相当する差分を
patch ファイルとして生成する。`strategy: patch` を明示しているにもかかわらず
ローカルファイルがないのは patch ファイルの作成漏れを意味するため、
upstream との完全な差分を生成して確認・編集できる状態にする。

**upstream もローカルも変更しない**: patch ファイルのみ更新する。

典型的な利用タイミング:

- プロジェクト新規セットアップ時(patch ファイルをゼロから生成)
- `graft sync` でコンフリクト warn が出た後
- upstream の変更を取り込む PR のレビュー前

---

## 2. upstream manifest と local overlay

### 2.1 概要

マニフェストを各 downstream リポジトリに個別に配置するのではなく、
upstream テンプレートリポジトリ (例: `naa0yama/boilerplate-rust`) で一元管理し、
downstream は `--upstream-manifest` でそれを参照する構成を取れる。

downstream が特殊なカスタマイズを必要とする場合は、ローカルに overlay マニフェストを
置くことで upstream の定義を選択的に上書き・除外できる。

### 2.2 upstream manifest 参照形式

```
owner/repo@ref:path
```

| 要素         | 説明                                        | 省略時のデフォルト |
| ------------ | ------------------------------------------- | ------------------ |
| `owner/repo` | upstream リポジトリ (`owner/name` 形式必須) | —                  |
| `@ref`       | ブランチ、タグ、またはコミット SHA          | `HEAD`             |
| `:path`      | リポジトリ内のマニフェストファイルパス      | (省略不可)         |

**例:**

```
naa0yama/boilerplate-rust@main:.github/graft/config.yaml
naa0yama/boilerplate-rust@v1.0.0:.github/graft/config.yaml
naa0yama/boilerplate-rust:.github/graft/config.yaml  # ref 省略 → HEAD
```

### 2.3 マニフェスト解決ロジック

1. `--upstream-manifest` が指定された場合:
   - 指定された参照を `gh api` で取得し、YAML をパースする
   - `--manifest` に指定されたローカルファイルが存在すれば、`merge_overlay` を適用する
   - ローカルファイルが存在しない場合は upstream マニフェストをそのまま使用する
2. `--upstream-manifest` が指定されない場合 (自動検出):
   - TTY 環境かつ `--yes` なし: `gh api repos/{owner}/{repo}` で fork 親または
     テンプレート親を自動検出し、使用するか確認プロンプトを表示する
   - ユーザーが承諾した場合: 検出した `owner/repo@branch:manifest-path` を
     upstream マニフェストとして使用する (上記 1 と同じ流れ)
   - 非 TTY / `--yes` 指定 / 検出失敗 / ユーザー拒否: ローカルファイルのみを使用する

**自動検出の優先順位**: fork 親 (`parent` フィールド) > テンプレート親 (`template_repository`)

**高速パス**: GitHub Actions 環境では `GITHUB_REPOSITORY` 環境変数を参照し、
`gh repo view` の呼び出しを省略する。

### 2.4 マージ規則

`merge_overlay(upstream, local)` の適用規則:

#### `upstream:` ノード

local マニフェストが存在する場合、local の `upstream:` ノードが upstream のものを
**完全に置換**する。部分マージは行わない。

#### `spec:` ノード

フィールド単位で **local 優先**マージを行う。`Option<T>` フィールドは
local が `Some` のときのみ上書きし、`None` のときは upstream の値を継承する。

#### `files:` リスト

`path` を key にマージする:

| 状態                              | 結果                                |
| --------------------------------- | ----------------------------------- |
| upstream のみに存在               | upstream ルールをそのまま採用       |
| local のみに存在                  | local ルールを末尾に追加            |
| 両方に存在                        | local ルールで **完全置換**         |
| 両方に存在 かつ local が `ignore` | 当該 path をマージ結果から **削除** |
| local のみに存在 かつ `ignore`    | マージ結果に追加しない (無視)       |

### 2.5 strategy: ignore

`strategy: ignore` は **local overlay 専用**の戦略で、upstream で定義されたルールを
downstream が明示的に除外するために使う。

```yaml
# local overlay (.github/graft/config.yaml)
upstream:
  repo: naa0yama/boilerplate-rust
  ref: main
files:
  # upstream で定義された Cargo.toml の patch ルールを除外する
  - path: Cargo.toml
    strategy: ignore
  # upstream にないローカル固有のルールを追加
  - path: .project-config.json
    strategy: create_only
```

**制約:**

- `source` フィールドは指定できない (`delete` と同じ制約)
- `patch` フィールドは指定できない
- ドリフト検知・sync の両対象から除外される (patch ファイルの存在チェックも対象外)

### 2.6 preserve_markers — ファイル内マーカーでブロックを保護

`preserve_markers` は `strategy: patch` または `strategy: replace` ルールに指定できるオプションフィールドで、
downstream のファイル内にマーカーコメントを書いてブロックを保護する。
**デフォルトは `true`**(省略時は保護が有効)。マーカー保護を無効にする場合のみ `false` を明示する。

```yaml
# .github/graft/config.yaml
files:
  # patch ファイルを使いつつマーカーも保護(デフォルトで有効)
  - path: Cargo.toml
    strategy: patch

  # マーカー保護だけが目的で patch ファイル不要の場合(デフォルトで有効)
  - path: .vscode/launch.json
    strategy: replace

  # マーカー保護を明示的に無効にする場合のみ false を指定
  - path: some/file.yaml
    strategy: replace
    preserve_markers: false
```

#### マーカー構文

ファイル内に以下の 2 行でブロックを囲む:

| 行に含まれるトークン | 役割               |
| -------------------- | ------------------ |
| `graft:keep-start`   | 保護ブロックの開始 |
| `graft:keep-end`     | 保護ブロックの終了 |

`gh-sync:keep-start` / `gh-sync:keep-end` はレガシートークンとして後方互換のため引き続き認識される。

コメント記号 (`#`, `//` 等) は無関係 — 行内にトークンが含まれているかどうかだけを判定する。
TOML (`#`)、Shell (`#`)、JSONC (`//`) のいずれでも使用できる。

例: `Cargo.toml` (TOML):

```toml
[workspace]
# graft:keep-start
members = ["crates/graft", "crates/graft-engine", "crates/graft-manifest"]
# graft:keep-end

[workspace.package]
# graft:keep-start
version = "0.2.1"
# graft:keep-end
edition = "2021"
```

例: `.vscode/launch.json` (JSONC):

```jsonc
{
	"configurations": [
		{
			// graft:keep-start
			"name": "Debug graft",
			"cargo": { "args": ["build", "--bin=graft"] }
			// graft:keep-end
		}
	]
}
```

#### 動作セマンティクス

| フェーズ                           | 動作                                                                                                                                                                                                                                                           |
| ---------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `sync --patch-refresh`             | upstream とローカルの**両方**のマーカーブロックを除去してから `diff -u` を実行する。生成される `.patch` ファイルにマーカー内の差分は含まれない。upstream 側もマーカーを "編集可能領域" として事前配置できる。                                                  |
| ドリフト検知 (`sync` / `ci-check`) | upstream とローカルの**両方**のマーカーブロックを除去した状態で patch 適用結果と比較する。マーカー内の変化は `Unchanged` と判定される。                                                                                                                        |
| 書き戻し (`sync --apply-files`)    | upstream に patch を適用した結果に、ローカルにマーカーブロックがあればそれを復元して書き込む。ローカルにマーカーブロックがない場合は upstream のマーカーブロックを伝播させる(初回同期でマーカー構造を継承)。マーカー内の内容は upstream 変更の影響を受けない。 |

#### エラー

| 状況                                                 | 動作                            |
| ---------------------------------------------------- | ------------------------------- |
| `keep-start` のみで `keep-end` がない (またはその逆) | sync を停止してエラーを報告する |
| マーカーのネスト (`keep-start` 内に `keep-start`)    | sync を停止してエラーを報告する |

#### `strategy: ignore` との使い分け

| 方式                     | 対象                             | downstream のファイルが存在するか         |
| ------------------------ | -------------------------------- | ----------------------------------------- |
| `strategy: ignore`       | ファイル丸ごと除外               | upstream から取得しない                   |
| `preserve_markers: true` | ファイル内の特定ブロックだけ保護 | upstream と同期しつつ、保護ブロックは維持 |

**制約:**

- `strategy: patch` または `strategy: replace` でのみ有効。`create_only`、`delete`、`ignore` と組み合わせるとスキーマエラーになる。
- マーカーのネストは禁止。
- マーカーのペアが一致しない (orphan) 場合は sync が停止する。

### 2.7 GitHub Actions での使用例

**純 upstream 動作** (downstream に local manifest なし):

```yaml
- uses: naa0yama/graft@v0.1.3
  with:
    token: ${{ secrets.GITHUB_TOKEN }}
    upstream-manifest: naa0yama/boilerplate-rust@main:.github/graft/config.yaml
```

**local overlay あり** (downstream に上書きルールを定義):

```yaml
- uses: naa0yama/graft@v0.1.3
  with:
    token: ${{ secrets.GITHUB_TOKEN }}
    upstream-manifest: naa0yama/boilerplate-rust@main:.github/graft/config.yaml
    manifest: .github/graft/config.yaml
```
