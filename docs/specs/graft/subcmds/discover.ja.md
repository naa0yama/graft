# graft discover — downstream リポジトリの検出

`graft discover` は GitHub API を使って `--owner` 配下のリポジトリをスキャンし、
指定した upstream テンプレートリポジトリを `parent` または `template_repository`
として持つ downstream リポジトリを検出する。

## 1. 概要

- `--owner` 配下の全リポジトリを GitHub API で列挙する
- 各リポジトリの `parent` または `template_repository` が `--upstream-repo` と
  一致するものを downstream と判定する
- 検出結果を標準出力に 1 行 1 リポジトリ (`owner/repo` 形式) で出力する
- `--repo` で対象を絞り込める

## 2. コマンド仕様

```
graft discover [OPTIONS]

Options:
  --owner <OWNER>           スキャン対象の GitHub owner/org (必須)
  --upstream-repo <REPO>    上流リポジトリ。書式: [owner/]repo (必須)
                              owner 省略時は --owner を補完
  --repo <REPO>             対象リポジトリを絞る (複数指定可)
```

## 3. 出力形式

```
owner/repo-a
owner/repo-b
owner/repo-c
```

エラーがある場合は stderr に出力し、stdout には成功した検出結果のみを出力する。

## 4. 配布との関係

`graft discover` はリポジトリの検出のみを担う。検出した downstream リポジトリへ
upstream の変更を一括配布する操作は `distribute-upstream` Claude スキルが
`graft discover` の出力を受け取って実行する。

これにより長期クレデンシャル (GitHub App 秘密鍵など) を保存せず、
ローカルの `gh auth` と gitconfig で commit 署名・PR 作成が可能になる。
