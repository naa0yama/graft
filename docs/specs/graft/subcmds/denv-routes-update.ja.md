# graft denv routes-update — ブランチ対応 Traefik ルート再生成

## 1. 目的

devcontainer 内で稼働中に `git checkout` した際、`.githooks/post-checkout` から呼ばれ
現在ブランチ向けの Traefik file-provider YAML を再生成する。ホスト側ではなく
コンテナ内から実行する専用サブコマンド (`host_check()` なし)。

```
graft denv routes-update
```

フラグなし。すべての入力は環境変数から取得する。

## 2. 環境変数

| 変数                  | 用途                                                                           | 未設定時の動作 |
| --------------------- | ------------------------------------------------------------------------------ | -------------- |
| `TRAEFIK_MANAGED`     | `"1"` のときのみ動作。それ以外は early return                                  | warn + exit 0  |
| `TRAEFIK_PROJECT`     | プロジェクト名 (`graft denv up` が注入)                                        | warn + exit 0  |
| `TRAEFIK_DYNAMIC_DIR` | コンテナ内の Traefik dynamic dir パス (例: `/traefik-dynamic`)                 | warn + exit 0  |
| `HOSTNAME`            | Docker cid_short と一致。YAML ファイル名の `${project}-${hostname}.yml` に使用 | warn + exit 0  |
| `GIT_WORK_TREE`       | ブランチ解決の起点。未設定時は `PWD`、それも未設定なら `.` にフォールバック    | フォールバック |

`graft denv up` は `TRAEFIK_MANAGED=1` / `TRAEFIK_PROJECT` / `TRAEFIK_DYNAMIC_DIR` を
自動で `--env` 転送する。post-checkout hook はこれらの継承を前提として動作する。

## 3. 動作フロー

1. `TRAEFIK_MANAGED`, `TRAEFIK_PROJECT`, `TRAEFIK_DYNAMIC_DIR` を検査。未設定なら warn + exit 0
2. `HOSTNAME` を検査。空なら warn + exit 0
3. `hostname -I` 相当でコンテナ IP を取得。空またはエラーなら warn + exit 0
4. `git -C $workspace branch --show-current` で現在ブランチを取得し `normalize_branch` を適用
5. `.devcontainer/devcontainer.json` を読み `portsAttributes` を取得。読み込み失敗なら warn + exit 0
6. `portsAttributes` が空なら warn + exit 0
7. `write_routes(hostname, project, branch, ip, ports, dynamic_dir)` を呼び
   `${TRAEFIK_DYNAMIC_DIR}/${TRAEFIK_PROJECT}-${HOSTNAME}.yml` を書き込む

## 4. エラーポリシー

hook から呼ばれるため、環境や中間データの欠落は **non-fatal** で warn + exit 0 とし、
`git checkout` を止めない。

| 事象                                          | 結果           |
| --------------------------------------------- | -------------- |
| Traefik 関連 env 未設定                       | warn + exit 0  |
| `HOSTNAME` 未設定                             | warn + exit 0  |
| IP 解決失敗 / 空                              | warn + exit 0  |
| `.devcontainer/devcontainer.json` 不在 / 不正 | warn + exit 0  |
| `portsAttributes` が空                        | warn + exit 0  |
| `write_routes` 失敗 (I/O エラー)              | error + exit 1 |

## 5. 出力

`${TRAEFIK_DYNAMIC_DIR}/${TRAEFIK_PROJECT}-${HOSTNAME}.yml` を上書きする。
Traefik は file provider の変更を検知し即時ルート切り替えする。YAML 生成規則は
`docs/specs/components/traefik-routing.ja.md` を参照。

## 6. 呼び出し元

主に `.githooks/post-checkout` から branch checkout (`$3=1`) 時にのみ呼ぶ:

```bash
if [ "${3:-0}" = "1" ] && [ "${TRAEFIK_MANAGED:-}" = "1" ]; then
    if command -v graft &>/dev/null; then
        graft denv routes-update 2>/dev/null || true
    fi
fi
```

`graft` 未インストール環境では silent skip。手動実行 (`graft denv routes-update`)
も同等に動作する。

## 7. Worktree 対応

worktree = 別 workspace = 別 `graft denv up` = 別コンテナ = 別 `HOSTNAME`。
`${project}-${HOSTNAME}.yml` のファイル名で自然に分離されるため、追加ハッシュや
ロックは不要。同一コンテナが複数ブランチを同時に持つケースは git checkout が
atomic である以上発生しない。
