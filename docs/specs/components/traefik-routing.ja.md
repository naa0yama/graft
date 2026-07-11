# Traefik ルーティング — ブランチ対応 devcontainer ルート

## 1. 全体像

`graft denv` は devcontainer をホスト側で起動する際に Traefik file provider の
dynamic YAML を生成し、ブランチ名を含む FQDN でコンテナ内サービスに到達できる
リバースプロキシを構築する。ブランチ切替時はコンテナ内の post-checkout hook から
`graft denv routes-update` が呼ばれ YAML を書き換える。

```
host                        container
──────────────────────      ─────────────────────────
graft denv up               .githooks/post-checkout
  ├─ traefik_dynamic_dir/   ──────► graft denv routes-update
  │    ${proj}-${cid}.yml           ├─ read devcontainer.json
  │                                 ├─ resolve IP + branch
  └─ Traefik (file provider)        └─ write same YAML
       ▲ auto-reload
```

## 2. YAML ファイル命名

- パス: `${TRAEFIK_DYNAMIC_DIR}/${TRAEFIK_PROJECT}-${HOSTNAME}.yml`
- `HOSTNAME` = Docker が付与する `cid_short`
- ホスト側の `traefik_dynamic_dir()` (例: `~/.config/traefik/dynamic`) が
  コンテナ内の `TRAEFIK_DYNAMIC_DIR` (例: `/traefik-dynamic`) に bind mount 済み
- 1 コンテナ = 1 YAML。複数コンテナは HOSTNAME でファイルが分かれるため衝突しない

## 3. ルーター / サービス命名と FQDN

`.devcontainer/devcontainer.json` の `portsAttributes` に含まれる各 port に対し:

- router 名 / service 名: `p{port}-{branch}--{project}`
- 到達 FQDN: `p{port}.{branch}.{project}.localhost`
- entryPoint: `web`
- loadBalancer server: `http://{container_ip}:{port}`

例: project `myproj`, branch `main`, port `8080`, IP `172.20.0.2` (RFC 1918):

```
router  p8080-main--myproj
service p8080-main--myproj  →  http://172.20.0.2:8080
Host    p8080.main.myproj.localhost
```

## 4. ブランチ正規化

`normalize_branch(raw)` により DNS ラベルとして安全な文字列にする:

- 小文字化
- 英数字以外はハイフンに置換
- 連続ハイフンは 1 個にまとめる
- 先頭 / 末尾のハイフンを trim
- 63 文字に truncate (DNS ラベル最大長)

例: `feature/My-Branch` → `feature-my-branch`, `feat//test` → `feat-test`。

## 5. Worktree 分離

worktree ごとに `graft denv up` すると別コンテナが起動し、各コンテナは異なる
`HOSTNAME` (cid_short) を持つ。YAML ファイル名に `HOSTNAME` を含めるため、
複数 worktree が同じホスト上で稼働しても互いのルート定義を上書きしない。

## 6. ライフサイクル

| 契機                                        | 動作                                                           |
| ------------------------------------------- | -------------------------------------------------------------- |
| `graft denv up`                             | 起動時に `write_routes` で YAML を初期生成                     |
| `graft denv exec` (既存コンテナ再接続)      | 既存 YAML を再利用。差分なければ書き換えなし                   |
| コンテナ内 `git checkout <branch>` (branch) | post-checkout hook → `graft denv routes-update` が YAML 上書き |
| `graft denv down`                           | `remove_routes` で当該 YAML を削除。Traefik のルートも消える   |

## 7. post-checkout hook 連携

`.githooks/post-checkout` は branch checkout (`$3=1`) かつ `TRAEFIK_MANAGED=1` の
とき `graft denv routes-update` を呼ぶ。`graft` 未インストール環境や失敗時は
`|| true` により hook 全体は成功扱いにする。詳細は
`docs/specs/graft/subcmds/denv-routes-update.ja.md` を参照。

## 8. statusline jq フィルタ

Claude statusline (`~/.claude/scripts/statusline-command.sh`) は Traefik API を
ポーリングし、自分のブランチに紐づくルートのみを表示する。jq フィルタは
router 名にブランチを埋め込む命名 (`p{port}-{branch}--{project}`) を利用して
現在ブランチだけを抽出する:

```bash
| jq -r --arg proj "$_traefik_project" --arg branch "$_cur_branch" '
    [.[] |
    select(.name | test("^p[0-9]+-" + $branch + "--" + $proj + "@file$")) |
    select(.serverStatus | to_entries | map(.value == "UP") | any) |
    .name | split("-")[0]] | join(" ")'
```

キャッシュキーにもブランチを含めることで別ブランチのキャッシュとの混線を防ぐ:

```
${CLAUDE_CACHE_DIR}/.claude-traefik-api-${_traefik_cid}-${_cur_branch}
```

statusline は YAML を直接書かず、生成は `graft denv up` / `routes-update` に
一元化されている (二重管理と競合上書きの回避)。

## 9. 環境変数まとめ

`graft denv up` がコンテナへ転送する Traefik 関連 env:

| 変数                  | 値の例                                         | 参照者                           |
| --------------------- | ---------------------------------------------- | -------------------------------- |
| `TRAEFIK_MANAGED`     | `"1"`                                          | routes-update guard, hook guard  |
| `TRAEFIK_PROJECT`     | プロジェクト名                                 | YAML 生成, router 名, statusline |
| `TRAEFIK_DYNAMIC_DIR` | `/traefik-dynamic`                             | YAML 出力先                      |
| `TRAEFIK_API_BASE`    | `http://host.docker.internal:{dashboard_port}` | statusline API ポーリング        |
