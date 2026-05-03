# Design: distribute-upstream skill + graft refactor

## Problem

`graft distribute` pushed upstream file changes to downstream repos via GitHub
GraphQL API with a GitHub App token. This requires storing a long-lived private
key in GitHub Actions secrets — a supply chain attack risk where a leaked key
allows token generation indefinitely.

The existing `graft sync file` command already applies upstream changes to a
local working tree. All that is needed is a discovery command and a skill that
orchestrates per-repo work locally.

## Goals

- Distribute upstream file changes without storing long-lived credentials anywhere.
- Use local gitconfig for commit signing automatically (SSH or GPG).
- Process repos in parallel via subagents to save time.
- Let Claude review each repo's diff and ask the user before committing.
- Remove dead code from `graft distribute`.

## Non-Goals

- Running distribution in CI / GitHub Actions.
- Handling repos not accessible via `gh auth` of the running user.

## Approach

Two coordinated changes:

### 1. Rust: replace `graft distribute` with `graft discover`

The multi-API discovery logic (org/user fallback, fork + template detection,
pagination) is non-trivial and worth keeping as a CLI command. Everything else
in `distribute/` is dead code once the skill takes over.

**Delete:**

- `distribute/content.rs` — file diff computation via API (replaced by `graft sync`)
- `distribute/github.rs` — GraphQL commit, PR creation, upsert_branch_ref
- `distribute/mod.rs` — orchestration and `DistributeReport`/`RepoOutcome` types
- `distribute/cli.rs` — `DistributeArgs`
- `Distribute` variant from `crates/graft/src/cli.rs`

**Keep and promote:**

- `distribute/discovery.rs` → move to `discover/mod.rs`, expose as `graft discover`

**New subcommand: `graft discover`**

```
graft discover --owner <owner> --upstream-repo <upstream> [--repo <filter>]...
```

Outputs a newline-separated list of `owner/repo` names to stdout (one per line).
Tracing goes to stderr only. This keeps the output trivially parseable by the
skill via `while IFS= read -r repo`.

### 2. Skill: `distribute-upstream`

Orchestrates the full workflow using `graft discover`, `graft sync file`, and
`gh` CLI. Dispatches one subagent per repo so all repos run in parallel.

**Top-level flow:**

```
skill:
  1. Ask user for upstream repo (e.g. naa0yama/boilerplate-rust)
  2. graft discover --owner <owner> --upstream-repo <upstream>
     → list of downstream repos
  3. Dispatch one subagent per repo (parallel)
  4. Collect results, report to user
```

**Per-repo subagent flow:**

```
subagent:
  1. Resolve work dir: $(ghq root)/github.com/<owner>/<repo>
     If absent: ghq get github.com/<owner>/<repo>
  2. git fetch origin
     git checkout -B graft/distribute origin/main
  3. graft sync file \
       --upstream-manifest "<upstream>@main:.github/graft/config.yaml" \
       --yes
  4. git diff HEAD  →  if empty: report "no changes", exit
  5. Show diff, AskUserQuestion: commit / skip / abort
  6. git add -A
     git commit -m "chore(graft): sync upstream files from <upstream>"
     (signing automatic via gitconfig)
     git push origin graft/distribute
  7. gh pr create \
       --title "chore(graft): sync upstream files from <upstream>" \
       --body  "<generated from git diff main...graft/distribute>" \
       --head  graft/distribute \
       --base  main
  8. Return PR URL to parent
```

## Implementation Notes

### `graft discover` CLI args

```
--owner          GitHub owner/org to scan
--upstream-repo  Upstream template repo (format: [owner/]repo[@ref[:path]])
--repo           Filter to specific repos (repeatable, optional)
```

Output: one `owner/repo` per line to stdout. Mirrors the existing
`discover_downstream_repos()` signature in `discovery.rs`.

### Manifest resolution in downstream repos

`graft sync file --upstream-manifest` fetches the upstream manifest and merges
it with the local `.github/graft/config.yaml` if present. If the downstream repo
has no local manifest, only the upstream manifest is used. This handles both
fork repos (may have a local overlay) and template repos (may have none).

### Parallel subagents

The skill dispatches all per-repo subagents in a single Agent tool call batch.
Each subagent is independent: it owns its `ghq` working directory and does its
own `AskUserQuestion`. The parent skill collects outcomes and prints a summary.

## File locations

- New subcommand: `crates/graft/src/discover/` (mod.rs + cli.rs)
- Skill: `.claude/skills/distribute-upstream/SKILL.md` (project-specific)
- Agent instructions: `.claude/skills/distribute-upstream/agent.md`

## Testing Strategy

**Rust (`graft discover`):**

- Unit tests in `discover/mod.rs` using existing `MockRunner` pattern from
  `discovery.rs` (tests migrate with the code).

**Skill:**

- Manual end-to-end: change a file in upstream template → run skill → verify
  signed commits and PRs appear in downstream repos.

## Rust code changes summary

| File                      | Action                                           |
| ------------------------- | ------------------------------------------------ |
| `distribute/discovery.rs` | Move → `discover/mod.rs`                         |
| `distribute/cli.rs`       | Delete                                           |
| `distribute/content.rs`   | Delete                                           |
| `distribute/github.rs`    | Delete                                           |
| `distribute/mod.rs`       | Delete                                           |
| `crates/graft/src/cli.rs` | Remove `Distribute`, add `Discover`              |
| `discover/cli.rs`         | New: `DiscoverArgs`                              |
| `discover/mod.rs`         | New: thin CLI wrapper over moved discovery logic |
