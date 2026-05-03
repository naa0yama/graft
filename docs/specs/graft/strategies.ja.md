# graft — 戦略

戦略は4種類: `replace`, `create_only`, `delete`, `patch`。
部分適用のニーズは `patch` 戦略で統一する。
patch ファイルは `graft sync --patch-refresh` で自動生成できるため、
マニフェスト内に複雑なマーカー記法を持ち込む必要がない。

全戦略の中核は純関数として定義する:

```
fn apply(rule, upstream_content, local_content) -> Result<StrategyResult>
```

これによりファイルシステムやネットワークアクセスなしでのユニットテストが可能になる。

## 1. `replace`

upstream バージョンでローカルファイルを置換する。**ファイル単位のみ対応**。

```yaml
- path: .clippy.toml
  strategy: replace
```

アルゴリズム:

1. upstream ファイルの内容を取得
2. ローカルファイルを読み取り(存在する場合)
3. 比較: 同一なら `Unchanged` を返す
4. upstream の内容をローカルパスに書き込み
5. 必要に応じて親ディレクトリを作成
6. `Changed` を返す

ディレクトリ同期は対応しない。ディレクトリ以下の各ファイルをルールとして個別に列挙する。
(`gh api` のディレクトリレスポンス判定、ローカルとの型不一致処理など実装コストが高く、
どのファイルを管理対象とするかはマニフェストで明示すべきであるため)

### upstream 404 の処理

upstream パスが 404 を返した場合: **warning** メッセージ付きで `Skipped` を返す。

```
upstream not found: {path} (use 'delete' strategy to remove local file)
```

ローカルファイルは変更も削除もしない。
これは gh-infra の障害モードを回避するための意図的な設計判断。gh-infra では 404 が
CLI 全体のエラー終了を引き起こし、後続ルールの実行を阻害していた。
削除は `delete` 戦略による明示的なアクションでなければならない。

### エラー条件

- `gh` CLI エラー(404 以外) → エラー

## 2. `create_only`

ローカルパスが存在しない場合のみ upstream からコピーする。

```yaml
- path: .github/project-config.json
  strategy: create_only
```

アルゴリズム:

1. ローカルパスが存在するか確認
2. 存在する → `Skipped` を返す(理由: "already exists")
3. upstream ファイルの内容を取得
4. ローカルパスに書き込み(必要に応じて親ディレクトリを作成)
5. `Changed` を返す

ファイルのみ対応。ディレクトリパスで `create_only` を使用するとエラー
(曖昧なセマンティクス: 一部のファイルが存在し一部が存在しない場合の扱いが不明確)。

**`--ci-check` での drift 判定**: ファイルの **存在のみ** を確認する。
ファイルが存在すれば `OK`(中身が upstream と異なっていても drift としない)。
ファイルが存在しない場合は warning を出力して最終的に exit 1。

```
[WARN]   .github/graft/config.yaml (create_only): file not found
```

`create_only` は「初回作成後はプロジェクト側で管理」する戦略のため、
内容の差分を drift とみなすと戦略の意図に反する。

### エラー条件

- パスがディレクトリを指している → エラー
- upstream パスが存在しない → エラー

## 3. `delete`

ローカルファイルまたはディレクトリを明示的に削除する。upstream fetch は行わない。

```yaml
- path: .github/rulesets
  strategy: delete
- path: .github/labels.json
  strategy: delete
```

この戦略が存在する理由: `replace` は upstream が 404 を返した場合に意図的にローカル
ファイルを削除しない。削除は明示的な宣言アクションでなければならない。これにより
upstream の再編成や一時的な API 障害による意図しないデータ損失を防ぐ。

**gh-infra からの教訓**: gh-infra は削除に `source: /dev/null` と `reconcile: mirror` を
使用していたが、CLI が「ファイルが意図的に削除された」と「API エラー」を区別する必要があった。
正当に削除された upstream ファイルが 404 を返した際、CLI は削除を実行する代わりに
エラー終了した。`delete` を独立した戦略として分離することで、この曖昧さを完全に
排除する — upstream fetch を行わないため、誤処理される 404 が発生しない。

アルゴリズム:

1. ローカルパスが存在するか確認
2. 存在しない → `Skipped` を返す(理由: "not found")
3. ファイル → ファイルを削除
4. ディレクトリ → 再帰的に削除
5. `Changed` を返す

### エラー条件

- アクセス権限エラー → エラー
- (他のエラーなし: パスが存在しない場合は `Skipped` であり、エラーではない)

## 4. `patch`

upstream ファイルを取得し、ローカルの unified diff パッチを適用してから結果を書き込む。

```yaml
# patch フィールド省略(推奨) — 慣例パスを自動解決
- path: Cargo.toml
  strategy: patch
```

### アルゴリズム

1. `patch` フィールドが省略されている場合、慣例パス `.github/graft/patches/<path>.patch` を使用
2. upstream ファイルの内容を取得
3. upstream の内容を一時ファイルに書き込み
4. 一時ファイルに `patch -p0 --no-backup-if-mismatch < patchfile` を実行
5. パッチ適用後の結果を読み取り
6. `preserve_markers: true` の場合、upstream とローカルファイルの**両方**からマーカーブロックを除去した状態で比較
7. 同一 → `Unchanged` を返す
8. `preserve_markers: true` の場合、ローカルにマーカーブロックがあればそれをパッチ結果に復元して書き込み。ローカルにマーカーブロックがない場合は upstream のマーカーブロックを伝播させて書き込み(初回同期時のマーカー構造の継承)
9. パッチ結果をローカルパスに書き込み
10. `Changed` を返す

### パッチファイル形式

`diff -u` で生成される標準的な unified diff 形式:

```diff
--- a/Cargo.toml
+++ b/Cargo.toml
@@ -1,6 +1,6 @@
 [workspace]
 resolver = "3"
 members = [
-    "crates/graft",
+    "crates/dtvmgr",
 ]
```

### パッチファイルの作成

patch ファイルは手書きせず `graft sync --patch-refresh` で生成する(subcmds/sync.md §4 参照)。
生成後に内容を確認してコミットする。

### コンフリクト時の挙動

コンフリクト(パッチ適用失敗)は **エラーではなく warning** として扱う:

- `[WARN] Cargo.toml (patch): conflict detected — skipped` を出力
- `patch` コマンドの stderr をそのまま warning メッセージに含める
- ファイルへの書き込みは行わず `Skipped(conflict)` を返す
- **後続ルールの処理は継続する** — check/diff は最後まで実行する
- 全ルール完了後、コンフリクトが1件以上あれば exit 1

これにより `--dry-run` や `--ci-check` 実行時でも全ルールの状態を一括確認できる。
コンフリクトが発生した場合は manifest-schema.md §8 の手順で patch ファイルを再生成する。

### エラー条件

- upstream パスが存在しない → エラー
- patch ファイルがローカルに存在しない → エラー(バリデーション Stage 2 で検出)
- `patch` コマンドが見つからない → エラー
- fuzz 付きでパッチ適用 → warning(エラーではない、適用は続行)

## 5. 戦略の結果型

各戦略は以下のいずれかを返す:

| ステータス        | 意味                                                                             | 終了コードへの影響 |
| ----------------- | -------------------------------------------------------------------------------- | ------------------ |
| `Changed`         | ファイルが変更された(dry-run では変更される)                                     | —                  |
| `Unchanged`       | ファイルは既に期待状態と一致                                                     | —                  |
| `Skipped(reason)` | ルールが意図的に適用されなかった(例: `create_only` で既存ファイル、upstream 404) | —                  |
| `Conflict`        | patch 適用がコンフリクト — warning を出力、書き込みをスキップ、処理は継続        | exit 1             |
| `Error(reason)`   | ルールの適用に失敗                                                               | exit 1             |

## 6. 戦略選択ガイド

| シナリオ                                                                  | 戦略          |
| ------------------------------------------------------------------------- | ------------- |
| CI ワークフロー、lint 設定など完全コピーが必要                            | `replace`     |
| 初回作成後はプロジェクト側でカスタマイズする設定                          | `create_only` |
| テンプレートで廃止されたレガシーファイル                                  | `delete`      |
| プロジェクト固有の変更が必要(workspace、パッケージ名、部分オーバーライド) | `patch`       |
