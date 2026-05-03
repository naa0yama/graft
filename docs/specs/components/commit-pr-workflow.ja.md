# graft — commit / PR ワークフロー

`graft sync` は **ファイルの書き込みのみ** 行い、commit / PR 操作は行わない。
main ブランチはプロテクトされているため、変更は必ずブランチ → PR 経由でマージする。

commit・PR の作成は skill 経由で Claude に委ねる。
これにより commit メッセージ・署名・PR 本文の内容が正確になり、
`graft sync` 側に git/gh の複雑な操作を持ち込まずに済む。

## 1. ローカル実行フロー

```
graft sync              # ファイルをローカルに書き込む
↓
(変更内容を確認)
↓
/commit スキル          # ブランチ作成・commit・PR 作成を Claude が担当
```

## 2. CI フロー (`--ci-check`)

```yaml
- name: drift detection
  run: graft sync --ci-check
  env:
    GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

乖離があれば exit 1 で CI を失敗させる。修正は別途ローカルで `graft sync` を実行する。
