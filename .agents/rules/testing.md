---
name: testing
description: Test rules, layout, and tooling for the quay Rust workspace.
paths:
  - "apps/cli/**/*.rs"
  - "apps/cli/**/Cargo.toml"
---

# Rust Testing — `apps/cli/`

Authoritative for the `tester` agent and for any code change that adds or modifies tests.

## 1. Layout

```
apps/cli/crates/<crate>/
├── src/
│   ├── lib.rs                # `#[cfg(test)] mod tests` for unit tests
│   └── <module>.rs           # unit tests live next to the code they test
├── tests/                    # integration tests — one file per feature area
│   ├── add.rs
│   ├── resolver.rs
│   └── common/
│       └── mod.rs            # shared fixtures, helpers
└── benches/                  # criterion benches (optional)
```

- **Unit tests** in the same file as the code:
  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;
      // …
  }
  ```
- **Integration tests** in `tests/<feature>.rs`. One Cargo target per file — `cargo test --test add` runs only `tests/add.rs`.
- **Shared fixtures** in `tests/common/mod.rs`. Import via `mod common;` in each integration test file.

## 2. Naming

- Test functions: `fn <verb>_<expectation>()` — e.g. `resolves_skill_from_remote_hub()`, `errors_when_hub_missing()`.
- No `test_` prefix. The `#[test]` attribute marks them; the prefix adds noise.
- One assertion concept per test. Multi-step setup is fine; testing five unrelated things in one function is not.

## 3. Required tooling

| Crate                        | Use case                                                        |
|------------------------------|------------------------------------------------------------------|
| `tempfile = "3"`             | Tempdirs for filesystem tests. Always.                           |
| `assert_cmd = "2"`           | CLI integration tests in `quay-cli/tests/`.                      |
| `predicates = "3"`           | Output assertions for `assert_cmd` (`predicate::str::contains`). |
| `insta = "1"`                | Snapshot tests for TUI output and complex serialized structures. |
| `proptest = "1"`             | Property tests for parsers and any code consuming untrusted input.|
| `pretty_assertions = "1"`    | Better diff output. Enable in dev-dependencies, gate with cfg.   |

Add via `[dev-dependencies]`. None of these may leak into runtime dependencies.

## 4. What to test where

| Code under test                         | Test type            | Lives in                                     |
|------------------------------------------|----------------------|----------------------------------------------|
| Pure function in `quay-core`             | unit                 | same file, `mod tests`                       |
| Multi-module logic in `quay-core`        | integration          | `crates/quay-core/tests/<feature>.rs`        |
| `quay <subcommand>` end-to-end           | CLI integration      | `crates/quay-cli/tests/cli_<subcommand>.rs`  |
| TUI screen rendering                     | snapshot             | `crates/quay-tui/tests/<screen>.rs` + `insta`|
| Resolver / parser invariants             | unit + property      | `mod tests` + `proptest!` block              |
| Lockfile JSON round-trip                 | unit                 | `mod tests`                                  |
| Hub git operations                       | integration          | `tests/git_<op>.rs` against tempdir + real git|

## 5. Fixtures

- **Real filesystem**, never mocked. Spawn `tempfile::TempDir` and write the layout you need.
- **Real git**, when needed. Shell out to `git` (or use `git2` if already in tree) inside a tempdir. Don't mock the git transport — that's exactly the integration we care about.
- **No network** in tests. Hub fixtures are vendored under `crates/<crate>/tests/fixtures/<hub-name>/` and copied into a tempdir at test start.
- **Deterministic time**. If a test cares about timestamps, inject a clock; never call `SystemTime::now()` in code under test.

## 6. Snapshot tests (`insta`)

- One snapshot per assertion. Use `insta::assert_snapshot!` for plain strings, `assert_yaml_snapshot!` for serialized structures, `assert_debug_snapshot!` for `Debug` output.
- Review with `cargo insta review`. Never blindly `cargo insta accept`.
- Snapshots live next to the test file under `snapshots/`. Commit them.

## 7. Property tests (`proptest`)

Use for any code that parses untrusted bytes — skill manifests, lockfiles, hub URLs, config files.

```rust
proptest! {
    #[test]
    fn lockfile_roundtrips(lock in any::<Lockfile>()) {
        let s = serde_json::to_string(&lock).unwrap();
        let parsed: Lockfile = serde_json::from_str(&s).unwrap();
        prop_assert_eq!(lock, parsed);
    }
}
```

If `proptest` finds a failing case, commit the regression in `proptest-regressions/` so CI catches it forever.

## 8. CI

CI runs:
1. `cargo fmt --all -- --check`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo test --workspace`
4. `cargo nextest run --workspace` (if available — faster + better failure output)

A red CI is a blocker. Do not merge over a failing test by `#[ignore]`-ing it.

## 9. Forbidden

- Mocking the filesystem. Use `tempfile`.
- Mocking git. Spawn real git in a tempdir.
- Network calls in tests. Vendor a fixture.
- `#[ignore]` to make CI green. If a test is genuinely flaky, file the flake and ask the team before ignoring.
- Tests that depend on test execution order. Each `#[test]` runs in isolation.
- `unwrap()` in test setup is fine; in test assertions, prefer `assert!`/`assert_eq!`/`pretty_assertions::assert_eq!`.

## See also

- Companion skill: [`.agents/skills/rust-testing/`](../skills/rust-testing/) (affaan-m)
- Style rules: [`code-style.md`](code-style.md)
- Tester persona: [`../agents/tester.md`](../agents/tester.md)
