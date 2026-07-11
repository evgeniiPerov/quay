---
name: code-style
description: Rust code style rules for the quay CLI workspace.
paths:
  - "apps/cli/**/*.rs"
  - "apps/cli/**/Cargo.toml"
---

# Rust Code Style — `apps/cli/`

These rules govern every `.rs` file under `apps/cli/`. CI enforces what's mechanically checkable; reviewers enforce the rest.

## 1. Formatting

- `cargo fmt --all` is the source of truth. CI rejects unformatted code (`cargo fmt --all -- --check`).
- Don't fight `rustfmt`. If you disagree with a result, it's still the answer.
- One `use` statement per item where it improves grep-ability; group nested `use` only when the group is short and homogeneous.

## 2. Linting

- `cargo clippy --workspace --all-targets -- -D warnings` must pass.
- Allowed lints opt-in only, at the smallest scope possible (item, not crate).
  ```rust
  #[allow(clippy::too_many_arguments)] // resolver has 8 inputs by design — see RFC-XX
  fn resolve(...) { }
  ```
- Never `#![allow(...)]` at the crate root without a comment explaining why.

## 3. Error handling

- **Libraries (`quay-core`)** define error enums with `thiserror`:
  ```rust
  #[derive(Debug, thiserror::Error)]
  pub enum ResolveError {
      #[error("hub {0} not found")]
      HubNotFound(String),
      #[error("git: {0}")]
      Git(#[from] git2::Error),
  }
  ```
- **Binaries (`quay`, `quay-cli`)** use `anyhow::Result` at the top of the call stack and add context with `.context()` / `.with_context()`.
- `?` is the default propagation. Raw `match` on `Result` is reserved for cases where both arms do meaningful work.
- **No `unwrap()` or `expect()`** in production code paths. Exceptions:
  - Tests, examples, build scripts.
  - Truly infallible operations where the panic is documented (`expect("static regex compiles")`).

## 4. Public API

- Every `pub` item gets a `///` doc comment explaining what it does and (where non-obvious) why.
- Module headers use `//!`.
- Mark public enums `#[non_exhaustive]` when future variants are likely (e.g. error enums).
- Re-export only what's intentionally part of the API surface. Don't `pub use foo::*`.

## 5. Idioms

- Prefer `&str` to `String` in arguments; prefer `impl AsRef<Path>` for path inputs.
- Borrow over clone. If you `.clone()`, leave a one-line comment if it's not obvious why.
- Iterators over indexed loops:
  ```rust
  // good
  let names: Vec<_> = skills.iter().map(|s| &s.name).collect();
  // bad
  let mut names = Vec::new();
  for i in 0..skills.len() { names.push(&skills[i].name); }
  ```
- `match` is exhaustive. `_ => unreachable!()` requires a comment justifying the invariant.
- `if let Some(x) = opt { ... } else { ... }` over `match` for two-arm Option/Result handling, unless both arms are non-trivial.

## 6. Logging

- Use `tracing` (not `log` directly, not `eprintln!`) for diagnostics in `quay-core` and `quay-cli`.
- `tracing::info!` for user-facing progress, `tracing::debug!` for development noise, `tracing::error!` only for genuine failures the user needs to act on.

## 7. Async

- `quay-core` is sync by default. If async is needed, isolate it behind a `tokio::main`-equipped binary entry, not deep in the library.
- If the project adopts `tokio`, follow [`wshobson/agents@rust-async-patterns`](https://skills.sh/wshobson/agents/rust-async-patterns) for cancellation, `Send`-bound, and runtime selection.

## 8. Dependencies

- Pin to minor in `Cargo.toml`: `serde = "1.0"`, not `"*"` and not `"=1.0.219"` unless you have a reason.
- `serde` features: `["derive"]` only unless you specifically need others.
- New dependencies justified in PR description. Reach for `std` first, then a tiny crate (`camino`, `tempfile`), then a heavyweight crate (`tokio`, `reqwest`) only if needed.
- Workspace `Cargo.toml` owns shared versions; member crates inherit via `workspace = true`.

## 9. Module layout

- One concept per file. `resolver.rs` is fine; `utils.rs` is not — split it.
- `mod.rs` is acceptable; equivalent `<modname>.rs` + `<modname>/` directory is also acceptable. Pick one per crate and stick with it.
- Tests colocated as `#[cfg(test)] mod tests` for unit tests; `tests/` directory for integration tests.

## 10. Comments

- Default to no comments. Code with good names is the documentation.
- Write a comment when the **why** is non-obvious: a workaround for an upstream bug, an invariant the type system can't express, a perf hack.
- Don't write comments that restate the code. Don't reference issue numbers without a brief description.

## 11. Forbidden

- `unsafe` code without a `// SAFETY:` comment proving the invariants. Reach for safe abstractions (`bytemuck`, `zerocopy`) first.
- `std::process::exit()` outside `main()`. Return `Result` and let the caller decide.
- `println!` / `eprintln!` for diagnostics. Use `tracing`. (User-facing CLI output is fine via `println!` in `quay-cli`.)
- `lazy_static!`. Use `std::sync::OnceLock` (1.70+) or `once_cell::sync::Lazy`.

## See also

- Companion skill: [`.agents/skills/rust-best-practices/`](../skills/rust-best-practices/) (apollographql)
- Test rules: [`testing.md`](testing.md)
