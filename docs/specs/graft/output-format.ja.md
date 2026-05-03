# graft — 出力形式

## 1. sync / dry-run 出力

ルールごとのステータス行に加え、変更があったファイルにはファイル単位の unified diff を表示する。

```
[CHANGED]  .github/workflows/ci.yml (replace)
--- a/.github/workflows/ci.yml
+++ b/.github/workflows/ci.yml
@@ -10,3 +10,4 @@
     - uses: actions/checkout@v4
     - uses: actions/setup-node@v4
+    - uses: actions/cache@v4

[OK]       .clippy.toml (replace)
[SKIPPED]  .github/project-config.json (create_only): already exists
[CHANGED]  Cargo.toml (patch)
--- a/Cargo.toml
+++ b/Cargo.toml
@@ -5,3 +5,3 @@
 members = [
-    "crates/graft",
+    "crates/dtvmgr",
 ]

[DELETED]  .github/labels.json (delete)
---
2 changed, 1 deleted, 1 up to date, 1 skipped
```

`--dry-run` 時は diff のみ表示し、ファイルへの書き込みは行わない。
diff 形式は `diff -u` 互換(ファイルパスのプレフィックスは `a/`, `b/`)。

## 2. validate 出力

```
[OK]    YAML syntax valid
[OK]    upstream.repo: naa0yama/boilerplate-rust
[OK]    rule[0] .github/workflows/ci.yml: replace
[OK]    rule[1] Cargo.toml: patch -> .github/graft/patches/Cargo.toml.patch (exists)
[FAIL]  rule[2] mise.toml: patch -> .github/graft/patches/mise.toml.patch (not found)
---
2 rules OK, 1 error
```

## 3. CI check 出力

drift が検出されたファイルには diff を表示する:

```
[DRIFT]  .github/workflows/ci.yml (replace): upstream has changes
--- a/.github/workflows/ci.yml (local)
+++ b/.github/workflows/ci.yml (expected)
@@ -10,3 +10,4 @@
     - uses: actions/checkout@v4
     - uses: actions/setup-node@v4
+    - uses: actions/cache@v4

[OK]     .clippy.toml (replace): up to date
[DRIFT]  Cargo.toml (patch): patched result differs from local
--- a/Cargo.toml (local)
+++ b/Cargo.toml (expected)
@@ -5,3 +5,3 @@
 members = [
-    "crates/graft",
+    "crates/dtvmgr",
 ]

---
2 files drifted, 1 up to date
```

**GHA 環境での追加出力** (`GITHUB_ACTIONS=true`):

アノテーション (stdout に出力、GHA がファイル位置表示に使用):

```
::error file=.github/workflows/ci.yml,title=graft drift::.github/workflows/ci.yml is out of sync with upstream
::error file=Cargo.toml,title=graft drift::Cargo.toml is out of sync with upstream
```

PR コメント本文 (PR に紐づく実行のみ `gh pr comment` で投稿):

````
## graft drift detected

| file | strategy | status |
| ---- | -------- | ------ |
| `.github/workflows/ci.yml` | replace | DRIFT |
| `Cargo.toml` | patch | DRIFT |

<details>
<summary>.github/workflows/ci.yml diff</summary>

```diff
@@ -10,3 +10,4 @@
     - uses: actions/checkout@v4
     - uses: actions/setup-node@v4
+    - uses: actions/cache@v4
```

</details>

Run `graft sync` locally to apply upstream changes.
````
