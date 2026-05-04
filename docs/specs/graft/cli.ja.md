# graft — CLI インターフェース

## 1. サブコマンド構造

```
graft [command]

Commands:
  sync        upstream テンプレートリポジトリからファイルを同期
  init        テンプレート同期設定ファイルを初期化
  issue-sync  upstream との乖離を検知し、追跡 GitHub Issue を管理
  discover    upstream を利用している downstream リポジトリを検出
  denv        devcontainer 環境の起動・停止・接続・状態確認
```

## 2. `graft sync` フラグ

| フラグ            | 短縮 | 型   | デフォルト                  | 説明                                           |
| ----------------- | ---- | ---- | --------------------------- | ---------------------------------------------- |
| `--manifest`      | `-m` | Path | `.github/graft/config.yaml` | マニフェストパス                               |
| `--dry-run`       | `-n` | bool | false                       | プレビューのみ(diff 出力、ファイル変更なし)    |
| `--validate`      |      | bool | false                       | マニフェストバリデーションのみ                 |
| `--ci-check`      |      | bool | false                       | drift detection                                |
| `--patch-refresh` |      | bool | false                       | patch ファイルを upstream との diff で自動生成 |

`gh` CLI が `GITHUB_TOKEN` 環境変数を自動的に使用する。
commit / PR 操作はこのコマンドでは行わない — skill 経由で Claude に委ねる。

## 3. フラグの競合ルール

- `--validate`, `--ci-check`, `--patch-refresh` は相互排他
- `--dry-run` は sync モード(デフォルト)のみ有効。他のモードでは無視する

## 4. `graft init` フラグ

| フラグ         | 短縮 | 型   | デフォルト                  | 説明                                                                           |
| -------------- | ---- | ---- | --------------------------- | ------------------------------------------------------------------------------ |
| `--repo`       | `-r` | str  | —(TTY の場合はプロンプト)   | 対象リポジトリ(`owner/name` 形式)                                              |
| `--ref`        |      | str  | `main`                      | 取得元の git ref                                                               |
| `--upstream`   |      | bool | false                       | upstream モード: `config.yaml` と `schema.json` を生成 (`--downstream` と排他) |
| `--downstream` |      | bool | false                       | downstream モード: `graft.yaml` ワークフローを生成 (`--upstream` と排他)       |
| `--select`     |      | bool | false                       | upstream のファイル一覧から対話的に選択 (`--upstream` 専用)                    |
| `--with-skill` |      | bool | false                       | `.claude/skills/graft/SKILL.md` を生成 (`--downstream` 専用)                   |
| `--output`     | `-o` | Path | `.github/graft/config.yaml` | 出力先パス (`--upstream` 専用)                                                 |
| `--force`      |      | bool | false                       | 既存ファイルを確認なしで上書き                                                 |

`--upstream` と `--downstream` はどちらか一方が必須(相互排他)。

- `--upstream` モード: `config.yaml` と `schema.json` を生成する。ワークフローや skill ファイルは生成しない。
- `--downstream` モード: `.github/workflows/graft.yaml` を生成する。`config.yaml` は生成しない。`--with-skill` を指定すると、さらに `.claude/skills/graft/SKILL.md` を生成し、marker 記法の使い方を Claude Code に伝えるスキルファイルを追加する。

config ファイルと同ディレクトリに `schema.json` (JSON Schema) を生成し、
yaml-language-server 対応エディタで補完・バリデーションが有効になる(`--upstream` モードのみ)。

config が不在の状態で `graft sync` を実行した場合、エラーメッセージに
`graft init` 実行のヒントを表示する。

## 5. `graft discover` フラグ

| フラグ            | 短縮 | 型        | デフォルト | 説明                                                                      |
| ----------------- | ---- | --------- | ---------- | ------------------------------------------------------------------------- |
| `--owner`         |      | str       | —          | スキャン対象の GitHub オーナー/Org (必須)                                 |
| `--upstream-repo` |      | str       | —          | upstream テンプレートリポジトリ。書式: `[owner/]repo` (必須)              |
| `--repo`          |      | str(複数) | —          | 対象 downstream リポジトリを絞り込む(繰り返し指定可: `--repo a --repo b`) |

`graft discover` は `--owner` 配下のリポジトリを GitHub API でスキャンし、
`parent` または `template_repository` が `--upstream-repo` と一致する
downstream リポジトリを一覧出力する (1 行 1 リポジトリ: `owner/repo` 形式)。

PR 作成などの実際の配布操作は `distribute-upstream` Claude スキルが担う。

## 6. `graft denv` サブコマンド

devcontainer 環境の起動・停止・接続・状態確認を行うサブコマンド群。
ホスト (WSL2 または Linux) 上で実行する必要があり、devcontainer 内からは実行できない。

```
graft denv [subcommand]

Subcommands:
  up       Traefik ルーティング付きで devcontainer を起動し、コンテナ内シェルに接続
  down     devcontainer を停止・削除し、Traefik ルートを解除
  exec     実行中の devcontainer に接続 (未起動の場合は up を実行)
  status   Traefik FQDN 付きで実行中の devcontainer 一覧を表示
  traefik  Traefik リバースプロキシのセットアップ
```

### `graft denv up` / `graft denv exec`

フラグなし。`$TMUX` 環境変数が設定されている場合、tmux ペインオプション
(`@role`, `@project-path`, `@pane-name`) を自動的に設定し、コマンド終了時にクリアする。
`@pane-name` は `<repo>:<branch>` 形式で設定される。

tmux が有効な場合、コンテナ起動時に以下の環境変数がコンテナへ転送される:

| 変数        | 説明                                                                      |
| ----------- | ------------------------------------------------------------------------- |
| `TMUX`      | tmux ソケットパスを含む接続文字列 (`socket_path,pid,pane_id`)             |
| `TMUX_PANE` | 現在のペイン ID (例: `%31`)。`set-option -p` によるペイン単位の設定に使用 |

tmux ソケットはコンテナにバインドマウントされ、コンテナ内のフックがホスト tmux の
`@pane-name` を直接更新できるようにする。

### `graft denv traefik setup`

```
graft denv traefik setup
```

Traefik バイナリをインストールし、systemd ユーザーサービスとして設定する (1 回だけ実行)。
`mise run traefik:setup` 経由で呼び出す。
