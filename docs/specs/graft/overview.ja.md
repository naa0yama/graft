# graft — 概要

## 1. 課題

`boilerplate-rust` はテンプレートリポジトリとして CI ワークフロー、lint 設定、devcontainer
設定などのインフラファイルを複数の downstream Rust プロジェクト(dtvmgr, chezmage,
dna-assistant, fterm)と共有している。

現在これらのファイルは `gh-infra` の File マニフェストで管理しているが、以下の限界がある:

- **テンプレート機能の不足**: Go テンプレートのプレースホルダ(`<% .Repo.Owner %>`)は
  メタデータのみ対応し、ファイル構造のカスタマイズができない
- **patch 未対応**: downstream がファイルの一部を変更する必要がある場合
  (例: `Cargo.toml` の workspace 設定)、表現する手段がない
- **未署名 commit**: `gh-infra` は GPG/SSH 署名なしで commit を作成するため、
  サプライチェーン検証に支障がある
- **404 エラー落ち**: upstream で削除されたファイルを `source: /dev/null` で宣言しても
  404 が返るとCLI がエラー終了し、後続ルールの実行が阻害される

## 2. 解決策

`graft sync` — `graft` CLI のサブコマンドとして、upstream テンプレートリポジトリから
downstream(fork)リポジトリへファイルを同期する。

設計判断:

- **pull 型モデル**: downstream リポジトリが同期マニフェストを所有する。
  upstream リポジトリは downstream への書き込みアクセスを持たない
- **`GITHUB_TOKEN` のみ**: GitHub App のインストールや PAT の発行は不要。
  fork 自身の `GITHUB_TOKEN`(GitHub Actions でデフォルトで利用可能)で
  public な upstream リポジトリの読み取りと fork 内での PR 作成が可能
- **署名付き commit**: `graft sync` はファイル書き込みのみ行い、commit は skill 経由で
  Claude が担当する。署名はユーザーの gitconfig 設定または GitHub Actions の
  commit API(自動 Verified)に委ねる
- **複数戦略**: ファイルの種類に応じて「完全コピー」から「プロジェクト固有の変更適用」
  まで異なるマージ方式が必要

## 3. 信頼モデル

**脅威**: 侵害された upstream テンプレートリポジトリが悪意ある CI ワークフローや
ビルド設定を downstream プロジェクトに送り込む。

**緩和策**:

1. **fork 所有のマニフェスト**: downstream リポジトリが同期対象と方法を明示的に宣言する。
   攻撃者が upstream のみを変更しても新しい同期対象を追加することはできない
2. **PR ベースの配信**: 同期結果は PR として配信され、直接 push はしない。
   メンテナがマージ前に差分をレビューする
3. **署名付き commit**: 同期 commit は downstream リポジトリの ID で署名される
   (upstream ではない)。明確な出所を確立する
4. **戦略による制御**: `patch` 戦略により、upstream ファイルの特定部分のみを
   受け入れることが可能で、upstream 変更の影響範囲を縮小する

**保護しないもの**:

- 侵害された downstream マニフェスト(攻撃者が `.github/graft/config.yaml` を
  変更できれば同期対象を制御できる)
- 受け入れた同期範囲内の悪意あるコンテンツ(`replace` でワークフローファイルを
  同期する場合、upstream ファイル全体が信頼される)

## 4. ユースケース

### UC-1: 新規 downstream プロジェクトの完全インフラ同期

`boilerplate-rust` テンプレートから新プロジェクトを作成。マニフェストに全 CI ワークフロー、
lint 設定、devcontainer ファイルを `replace` 戦略でリスト。週次スケジュール同期で
全ファイルを最新に保つ。

### UC-2: プロジェクト固有オーバーライド付き選択的同期

`dtvmgr` は CI ワークフローを共有するが、`Cargo.toml` の workspace 設定と
追加のリリースターゲットは独自。マニフェストでワークフローには `replace`、
`Cargo.toml` には `patch`、`project-config.json` には `create_only` を使用。

### UC-3: CI での drift detection

PR の CI ジョブで `graft sync --ci-check` を実行し、管理対象ファイルが期待状態から
乖離していないか検証。テンプレート管理すべきファイルへの意図しない手動編集を検出する。

### UC-4: PR でのマニフェストバリデーション

`.github/graft/config.yaml` が変更された際、CI ジョブで `graft sync --validate` を実行し、
マニフェストが正しい形式であることをマージ前に検証。

## 5. スコープ

### 目標

- GitHub リポジトリからの個別ファイル・ディレクトリの同期
- 4つの戦略: `replace`, `create_only`, `delete`, `patch`
- マニフェストバリデーション(スキーマ + 参照チェック)
- drift detection(ローカル状態と同期後の期待状態の比較)
- dry-run モード
- CI 連携向けの構造化出力

### 非目標

- push 型モデル(upstream → downstream)
- コンフリクト解決 UI やインタラクティブマージ
- git 履歴の同期や upstream commit attribution の保存
- GitHub 以外のホスティング対応(GitLab, Bitbucket)
- PR の自動マージ(人間によるレビューが意図的)
- バイナリファイルの patch 処理

## 6. アーキテクチャ

`graft` はワークスペース内の 3 クレートに分割されている。

```
graft-manifest   — 純粋データ型・スキーマ (I/O なし、Miri 対応)
        ↓
graft-engine     — ビジネスロジック・トレイト (外部バイナリ I/O なし、Miri 対応)
        ↓
graft            — CLI バイナリ (GhRunner 実装・コマンドライン解析)
```

### クレートの責務

| クレート         | 責務                                                                                   | I/O                                                    |
| ---------------- | -------------------------------------------------------------------------------------- | ------------------------------------------------------ |
| `graft-manifest` | マニフェストスキーマ型、YAML ロード、バリデーション                                    | YAML ファイル読み取りのみ                              |
| `graft-engine`   | 戦略実装、sync/validate/ci-check/patch-refresh モード、出力フォーマット                | なし(`GhRepoClient` / `GhRunner` トレイト経由で抽象化) |
| `graft`          | `GhRunner` トレイット本実装(`SystemGhRunner`)、`clap` コマンド解析、エントリーポイント | `gh` CLI 呼び出し・ファイルシステム書き込み            |

### `GhRunner` トレイト

`crates/graft/src/sync/runner.rs` で定義するトレイト。
`gh` CLI の呼び出しを抽象化し、テストでモック実装を注入できるようにする。

```rust
pub trait GhRunner: Send + Sync {
    fn run(&self, args: &[&str], stdin: Option<&[u8]>) -> anyhow::Result<GhOutput>;
}
```

本番環境では `SystemGhRunner` が `std::process::Command` で `gh` を起動する。
テストでは `MockGhRunner` を注入することで、`gh` CLI なしでユニットテストが実行できる。
