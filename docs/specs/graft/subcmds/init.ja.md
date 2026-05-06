# graft init — upstream 初期化

## graft init --upstream

`graft init --upstream` は `.github/graft/config.yaml` と `schema.json` を生成する。
GitHub Action や downstream 向けのファイルは生成しない。

- 非インタラクティブ (`stdin` が TTY でない) 時は `--select` を明示した場合のみ生成。
- TTY 時は `--select` または対話モードでファイルを選択できる。
- 既存ファイルがある場合は `--force` がなければ上書き確認を行う (非 TTY では bail)。

```bash
# 非インタラクティブ例
graft init --upstream --repo naa0yama/boilerplate-rust --select

# 出力先を変更する場合
graft init --upstream --repo naa0yama/boilerplate-rust --select --output .github/graft/config.yaml
```
