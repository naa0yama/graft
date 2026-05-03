# graft — upstream ファイルの取得

## 1. 取得方法

`gh` CLI 経由で取得する。API を直接叩かない。

### ファイル取得

```
gh api repos/{owner}/{repo}/contents/{path}?ref={ref} --jq '.content' | base64 -d
```

`gh` が `GITHUB_TOKEN` の認証・レート制限・エラーハンドリングを担う。

### ディレクトリ一覧

```
gh api repos/{owner}/{repo}/contents/{path}?ref={ref} --jq '.[].path'
```

エントリを再帰的に取得し、各ファイルを上記のファイル取得コマンドで読み込む。

### 404 判定

`gh api` は HTTP 404 時に非ゼロで終了する。終了コードで存在確認を行う。

## 2. レート制限

`gh` CLI が自動で処理する。個別の `X-RateLimit-*` ヘッダ管理は不要。
