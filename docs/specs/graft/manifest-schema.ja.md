# graft — マニフェストスキーマ

## ファイル配置

```
.github/graft/
├── config.yaml          # 同期マニフェスト(本セクションで定義)
└── patches/            # patch 戦略で使用する unified diff ファイル群
    ├── Cargo.toml.patch
    └── ...
```

マニフェストファイル: `.github/graft/config.yaml`(downstream/fork リポジトリ内)

### YAML vs TOML

| 観点                  | YAML                               | TOML                                          |
| --------------------- | ---------------------------------- | --------------------------------------------- |
| Rust との親和性       | `serde_yml` クレートが必要         | `toml` クレートは Cargo.toml と共通           |
| コメント              | 対応                               | 対応                                          |
| 型の厳格さ            | 文字列/bool の混在ミスが起きやすい | 型が明示的で strict                           |
| 配列の記述            | `- key: val` で簡潔                | `[[rules]]` ブロックで冗長になりがち          |
| `.github/` との一貫性 | GitHub Actions 等と統一感あり      | Rust プロジェクトの `Cargo.toml` と統一感あり |
| エディタ補完          | yaml-language-server が充実        | taplo 等が対応                                |

**現時点の判断**: rules の配列が長くなる本マニフェストでは `[[rules]]` 記法の冗長さがネック。
**YAML を採用**する。`serde(deny_unknown_fields)` + JSON Schema で
TOML の strict 型チェックと同等の堅牢性を確保する。

## 1. トップレベル構造

```yaml
upstream:
  repo: <owner>/<name> # 必須
  ref: <branch|tag|sha> # 任意、デフォルト: "main"

rules:
  - <rule>
...
```

### `upstream` オブジェクト

| フィールド | 型     | 必須   | デフォルト | 説明                                         |
| ---------- | ------ | ------ | ---------- | -------------------------------------------- |
| `repo`     | string | はい   | —          | `owner/name` 形式の GitHub リポジトリ        |
| `ref`      | string | いいえ | `"main"`   | 取得元の git ref(ブランチ、タグ、commit SHA) |

### トップレベルの制約

- `upstream` は必須
- `rules` は必須、空でない配列
- 未知のトップレベルフィールドは不可(strict パース)

## 2. ルール構造

```yaml
- path: <string>
  strategy: <replace|create_only|delete|patch>
  patch: <string> # patch 戦略のみ必須
```

### 共通フィールド(全戦略)

| フィールド | 型     | 必須 | 説明                                                   |
| ---------- | ------ | ---- | ------------------------------------------------------ |
| `path`     | string | はい | ローカル(downstream)側の相対パス。適用先となる         |
| `strategy` | enum   | はい | `replace`, `create_only`, `delete`, `patch` のいずれか |

`path` はローカルへの書き込み先を示す。upstream からの取得先は原則 `path` と同一だが、
`replace` / `create_only` では `source` で上書きできる。

### `replace` / `create_only` の追加フィールド

| フィールド | 型     | 必須   | デフォルト    | 説明                                                                     |
| ---------- | ------ | ------ | ------------- | ------------------------------------------------------------------------ |
| `source`   | string | いいえ | `path` と同じ | upstream リポジトリ内の取得元パス。upstream 側だけパスが異なる場合に指定 |

例: upstream では `templates/ci.yaml` に置かれているが、
ローカルには `.github/workflows/ci.yaml` として配置したい場合:

```yaml
- path: .github/workflows/ci.yaml
  strategy: replace
  source: templates/ci.yaml
```

`delete` は upstream を参照しないため `source` は不要。
`patch` は patch ファイルが `path`(= upstream 取得先)に対して生成されるため、
`source` ≠ `path` になると patch が成立しない — `source` は使用不可。

### `patch` 戦略の追加フィールド

| フィールド         | 型      | 必須   | デフォルト                           | 説明                                                                                                                                                                                      |
| ------------------ | ------- | ------ | ------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `patch`            | string  | いいえ | `.github/graft/patches/<path>.patch` | unified diff ファイルのパス(リポジトリルートからの相対)                                                                                                                                   |
| `preserve_markers` | boolean | いいえ | `true`                               | `false` にするとマーカー保護を無効にする。デフォルトでは `graft:keep-start` / `graft:keep-end` (または `gh-sync:keep-*` レガシー) で囲まれたブロックを保護する (`patch` / `replace` のみ) |

省略時は `path` をそのまま使い慣例パスを自動解決する。
慣例から外れる配置にしたい場合のみ明示指定する:

```yaml
# 省略形(慣例パスを自動使用)
- path: Cargo.toml
  strategy: patch

# 明示指定(慣例から外れる場合のみ)
- path: Cargo.toml
  strategy: patch
  patch: somewhere/Cargo.toml.patch
```

`patch` 戦略では `path` が upstream の取得先とローカルの適用先の両方を兼ねる。
取得先と適用先が一致しないと patch ファイルの文脈が崩れるため、`source` は指定不可。

### 組み合わせ禁止ルール

- `delete`: `source`, `patch` フィールド不可
- `patch`: `source` フィールド不可
- `create_only`, `delete`, `ignore`: `patch`, `preserve_markers` フィールド不可
- `replace`: `patch` フィールド不可 (`preserve_markers` は可)
- いずれのルールでも未知のフィールドはバリデーションエラー

## 3. パスの制約

`path` と `source` の両方に適用:

- 空文字列不可
- `/` 始まり不可(相対パス必須)
- `..` セグメント不可(パストラバーサル防止)
- `\` 不可(全プラットフォームで `/` を使用)
- 正規化済みであること(`./` プレフィックス不可、末尾 `/` 不可、二重 `//` 不可)

`path` のみ:

- 全ルールで一意であること

## 4. バリデーションレベル

バリデーションは2段階で実行する:

### Stage 1: スキーマバリデーション(オフライン、ネットワーク不要)

upstream を取得せずに実行できるチェック:

1. YAML 構文が有効
2. 全必須フィールドが存在
3. `upstream.repo` が `^[a-zA-Z0-9._-]+/[a-zA-Z0-9._-]+$` にマッチ
4. `strategy` が4つの許可値のいずれか
5. 戦略固有の必須フィールドが存在
6. 組み合わせ禁止ルールに違反しない(§2 参照)
7. 未知のフィールドがない
8. `path` および `source`(指定時)がパス制約を満たす(§3 参照)
9. `path` の重複がない

### Stage 2: 参照バリデーション(ローカルファイルシステム)

ローカルファイルの読み取りが必要なチェック:

1. `patch` 戦略: `patch` フィールド指定時はそのパスに、省略時は慣例パス(`.github/graft/patches/<path>.patch`)に patch ファイルが存在する

注意: upstream ファイルの存在はバリデーション時にはチェックしない。
ネットワークアクセスが必要なため、sync/ci-check 実行時のランタイムエラーとする。

## 5. 例

各戦略を1つずつ示す。

```yaml
upstream:
  repo: naa0yama/boilerplate-rust
  ref: main

rules:
  # replace: upstream と同一内容でローカルを上書き
  - path: .github/workflows/rust-ci.yaml
    strategy: replace

  # replace + source: upstream 側だけパスが異なる場合
  - path: .github/workflows/ci.yaml
    strategy: replace
    source: templates/ci.yaml

  # create_only: 存在しない場合のみ作成(以後プロジェクト側で管理)
  - path: .github/graft/config.yaml
    strategy: create_only

  # delete: テンプレートで廃止されたレガシーファイルを削除
  - path: .github/labels.json
    strategy: delete

  # patch: patch フィールド省略 → .github/graft/patches/Cargo.toml.patch を自動使用
  - path: Cargo.toml
    strategy: patch

  # patch (ネストしたパス): 同様に省略 → .github/graft/patches/.github/workflows/ci.yaml.patch
  - path: .github/workflows/ci.yaml
    strategy: patch
```

## 6. serde デシリアライゼーション

- `#[serde(deny_unknown_fields)]` で全レベルの未知フィールドを拒否
- `upstream.ref` は `#[serde(default)]` でデフォルト値 `"main"`
- `strategy` は小文字 enum (`#[serde(rename_all = "snake_case")]`)
- `patch` フィールドは `Option<String>` — 省略時は慣例パスを使用(§2 参照)

## 7. 将来の検討事項

- エディタ補完用の JSON Schema 生成(yaml-language-server 対応)
- ルール単位の `source` フィールドオーバーライド(マルチソース同期)
- `path` での glob パターン対応(例: `.github/workflows/*.yaml`)

## 8. patch ファイルの管理方針

`patch` 戦略で使用する patch ファイルは `.github/graft/patches/` 以下に、
対象ファイルのディレクトリ構造をそのまま維持して配置する:

```
.github/graft/patches/
├── Cargo.toml.patch
└── .github/
    └── workflows/
        └── ci.yaml.patch
```

パスを加工(スラッシュをドット等に変換)しない理由:

- 変換ロジックが実装・テストのコストになる
- ディレクトリ構造が失われると、どのファイルの patch かが一目でわからない
- `path` に `.patch` を付加するだけで取得できる単純なルールを維持する

マニフェスト内の `patch` フィールドは明示指定のため、慣例はあくまで推奨:

```yaml
- path: .github/workflows/ci.yaml
  strategy: patch
  patch: .github/graft/patches/.github/workflows/ci.yaml.patch
```

### patch ファイルの再生成

patch ファイルは upstream が更新されるとコンフリクトする可能性がある。
再生成が必要なタイミング:

- プロジェクト新規セットアップ時(patch ファイルがまだ存在しない)
- `graft sync` 実行時にコンフリクト warn が出た後
- upstream の変更を取り込む PR のレビュー前

`graft sync --patch-refresh` を実行する(subcmds/sync.md §4 参照)。
`strategy: patch` の全ルールに対して upstream との diff を `patches/` 以下に書き出すので、
内容を確認してコミットする。
