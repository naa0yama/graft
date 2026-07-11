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

| フラグ                | 短縮 | 型   | デフォルト                  | 説明                                                   |
| --------------------- | ---- | ---- | --------------------------- | ------------------------------------------------------ |
| `--manifest`          | `-m` | Path | `.github/graft/config.yaml` | マニフェストパス                                       |
| `--upstream-manifest` |      | str  | —                           | upstream マニフェスト参照 (`owner/repo@ref:path` 形式) |
| `--dry-run`           | `-n` | bool | false                       | プレビューのみ(diff 出力、ファイル変更なし)            |
| `--validate`          |      | bool | false                       | マニフェストバリデーションのみ                         |
| `--ci-check`          |      | bool | false                       | drift detection                                        |
| `--patch-refresh`     |      | bool | false                       | patch ファイルを upstream との diff で自動生成         |
| `--yes`               | `-y` | bool | false                       | 確認プロンプトをスキップして変更を適用                 |

`gh` CLI が `GITHUB_TOKEN` 環境変数を自動的に使用する。
commit / PR 操作はこのコマンドでは行わない — skill 経由で Claude に委ねる。

## 3. フラグの競合ルール

### `sync file`

- `--validate`, `--ci-check`, `--patch-refresh` は相互排他
- `--dry-run` は `--patch-refresh` および `--yes` と競合
- `--yes` は `--dry-run`, `--validate`, `--ci-check`, `--patch-refresh` と競合

### `sync repo`

- `--yes` は `--dry-run`, `--ci-check` と競合

## 4. `graft init` フラグ

| フラグ       | 短縮 | 型   | デフォルト                  | 説明                                                        |
| ------------ | ---- | ---- | --------------------------- | ----------------------------------------------------------- |
| `--repo`     | `-r` | str  | —(TTY の場合はプロンプト)   | 対象リポジトリ(`owner/name` 形式)                           |
| `--ref`      |      | str  | `main`                      | 取得元の git ref                                            |
| `--upstream` |      | bool | false                       | upstream モード: `config.yaml` と `schema.json` を生成      |
| `--select`   |      | bool | false                       | upstream のファイル一覧から対話的に選択 (`--upstream` 専用) |
| `--output`   | `-o` | Path | `.github/graft/config.yaml` | 出力先パス (`--upstream` 専用)                              |
| `--force`    |      | bool | false                       | 既存ファイルを確認なしで上書き                              |

`--upstream` は必須。

- `--upstream` モード: `config.yaml` と `schema.json` を生成する。ワークフローや skill ファイルは生成しない。

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
  up             Traefik ルーティング付きで devcontainer を起動し、コンテナ内シェルに接続
  down           コンテナを停止・削除・イメージ削除し、Traefik ルートを解除 (フルリセット)
  exec           実行中の devcontainer に接続 (未起動の場合は up を実行)
  status         Traefik FQDN 付きで実行中の devcontainer 一覧を表示
  routes-update  現在ブランチ用に Traefik ルートを再生成 (devcontainer 内で実行)
  traefik        Traefik リバースプロキシのセットアップ
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

### `graft denv routes-update`

```
graft denv routes-update
```

フラグなし。devcontainer 内から実行し、現在ブランチ用の Traefik file-provider YAML
(`${TRAEFIK_DYNAMIC_DIR}/${TRAEFIK_PROJECT}-${HOSTNAME}.yml`) を書き出す。
`.githooks/post-checkout` から branch checkout 時に自動呼び出しされる。

必要な環境変数 (`graft denv up` が自動注入): `TRAEFIK_MANAGED=1`, `TRAEFIK_PROJECT`,
`TRAEFIK_DYNAMIC_DIR`, `HOSTNAME`。workspace は `GIT_WORK_TREE` → `PWD` → `.` の順で解決。

env 未設定 / IP 未解決 / `devcontainer.json` 不在などの中間的な欠落は warn + exit 0
で hook を止めない。ファイル書き込み失敗のみ exit 1。詳細は
`docs/specs/graft/subcmds/denv-routes-update.ja.md` を参照。

### `graft denv traefik setup`

```
graft denv traefik setup
```

Traefik バイナリをインストールし、systemd ユーザーサービスとして設定する (1 回だけ実行)。
`mise run traefik:setup` 経由で呼び出す。
