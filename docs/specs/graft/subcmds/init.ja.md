# graft init — GitHub Action と初期化

## 1. GitHub Action

### 1.1 使い方

`action.yml` をリポジトリ直下に配置することで、下流リポジトリから
`uses: naa0yama/graft@<tag>` の形式で呼び出せる公開 Action として機能する。

実行シーケンス (固定・書き込みなし):

1. `graft sync file --validate` — マニフェストのスキーマ検証 (ローカル、upstream 接続なし)
2. `graft sync repo --ci-check` — リポジトリ設定のドリフト検知
3. `graft sync file --ci-check` — ファイルのドリフト検知

いずれかのステップが失敗すると Action は即時 exit 1 する (GitHub Actions のデフォルト fail-fast)。
ドリフトを **取り込む** 責務は持たず、検知のみ。

### 1.2 inputs

| name                | required | default                     | 説明                                                                        |
| ------------------- | :------: | --------------------------- | --------------------------------------------------------------------------- |
| `token`             |   yes    | —                           | `gh release download` と `gh` CLI 内部で使用するトークン                    |
| `version`           |    no    | `github.action_ref`         | ダウンロードするリリースタグ (例: `v0.1.3`)。SHA pin 時は明示必須           |
| `manifest`          |    no    | `.github/graft/config.yaml` | 同期設定ファイルのパス                                                      |
| `upstream-manifest` |    no    | —                           | upstream マニフェスト参照 (`owner/repo@ref:path` 形式)。詳細は sync.md 参照 |

## 2. graft init --downstream

`graft init --downstream` は `.github/workflows/graft.yaml` を生成する。
`config.yaml` / `schema.json` は生成しない (upstream モードの生成物)。

- 非インタラクティブ (`stdin` が TTY でない) 時は `--downstream` を明示した場合のみ生成。
- TTY 時はモード選択後にインタラクティブな確認プロンプトを表示する。
- 既存ファイルがある場合は `--force` がなければ上書き確認を行う (非 TTY では bail)。
- 埋め込まれるバージョンは `graft` 実行時の `CARGO_PKG_VERSION` (例: `v0.1.3`)。
- `--with-skill` を付けると `.claude/skills/graft/SKILL.md` を追加生成し、
  marker 記法 (`graft:keep-start` / `graft:keep-end` (または `gh-sync:keep-*` レガシー)) の使い方を Claude Code に伝える。

```bash
# 非インタラクティブ例 (ワークフローのみ生成)
graft init --downstream --repo naa0yama/boilerplate-rust --force

# skill ファイルも同時に生成する場合
graft init --downstream --repo naa0yama/boilerplate-rust --with-skill --force
```

生成される `.github/workflows/graft.yaml` の内容 (`--repo owner/repo` 指定時):

```yaml
# yaml-language-server: $schema=https://json.schemastore.org/github-workflow.json
name: graft check
on:
  push:
    branches: [main]
  pull_request:
    types: [opened, synchronize, reopened]
  schedule:
    - cron: "0 18 * * *" # daily at 03:00 JST
  workflow_dispatch:

permissions: {}

jobs:
  graft-check:
    name: graft-check
    runs-on: ubuntu-latest
    timeout-minutes: 10
    permissions:
      contents: read
    steps:
      - uses: actions/checkout@<sha> # vX.Y.Z
        with:
          persist-credentials: false
      - uses: naa0yama/graft@<version>
        with:
          token: ${{ secrets.GITHUB_TOKEN }}
          upstream-manifest: owner/repo@main:.github/graft/config.yaml
```
