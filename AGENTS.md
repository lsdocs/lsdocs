# AGENTS.md

Guidance for AI agents (and humans) working in lsdocs, one repository of
the lsdocs organization (https://github.com/lsdocs). It was split from the
lsdocs monorepo at commit b95f771; its history before that is the
monorepo's, filtered to these files. README.md says what it is and lists
its known limitations and TODO items.

## Working across repositories

Other lsdocs repositories are git dependencies
(`{ git = "https://github.com/lsdocs/<repo>", branch = "main" }`);
`Cargo.lock` pins their revisions (`nur bump [crate]` moves them).

- `nur link ../<repo>` builds against a local checkout (a `[patch]` in the
  untracked `.cargo/config.toml`); `nur unlink` undoes it.
- With `LSDOCS_ORG_DIR` set (e.g. `C:/dev`), Cargo fetches the
  organization's repositories from the local clones there (their
  committed `main`) instead of GitHub.
- `nur check-dupes` fails if a crate of the organization is built twice;
  `nur check-git-deps` fails on a dependency that is not a path inside
  this repository or a git dependency on the repository holding it
  (`scripts/org.nuon`), and on a committed `[patch]` table. `nur msrv`
  checks each crate on its own `rust-version`; `nur deny` runs
  cargo-deny (both are CI jobs of their own).

## Dev tasks

Dev tasks are nushell scripts run with `nur` (`nurfile`); `nur --list`
shows them. `nur ci` runs what CI runs: run it before committing. Tools:
`mise install` (see `.config/mise.toml`); Rust from rustup.

## Conventions

- Commits: Conventional Commits, `type(scope): summary`.
- Line endings: LF everywhere; scripts that write files write LF.
- Comments and docs: ASCII only.
- MSRV is per crate (`rust-version`); the workspace's is the base.
- Dependencies are declared in `[workspace.dependencies]` and used with
  `workspace = true`.
- Lints: the workspace lints apply to every crate; `missing_docs` is on;
  `unsafe_code` is denied.
- No panics outside tests: clippy denies `unwrap_used`, `expect_used` and
  `panic` in library, binary and example code. Tests opt in per module
  (`#![allow(clippy::unwrap_used)]`).
- Licensing: `Apache-2.0 OR MIT`.
- Publishing: merging release-plz's release PR publishes the bumped
  crates to crates.io (`.github/workflows/release.yml`); an `lsdocs/v*`
  tag's GitHub release gets the `lsdocs` binaries (`tag.yml`).
- Record what is left open in README.md's TODO section, and what the code
  does not do (or does differently) under Known limitations.

## This repository

| Path | What |
|---|---|
| `crates/host` | Plugin startup/shutdown, the build pipeline, the cache (`.lsdocs-cache/`, `src/cache.rs`), id and link normalization (`src/normalize.rs`). |
| `crates/cli` | The `lsdocs` binary (`build`, `serve`, `init`, hidden `plugin <name>`). End-to-end tests live here. |
| `tests/site` | The sample site the end-to-end snapshot test builds (`nur sample`, `nur snapshots`). |
| `examples/plugins/txt-parser` | A plugin written from `spec/wire.md` alone (standard-library Python, no SDK); `crates/cli/tests/no_sdk_plugin.rs` builds a site with it. |
| `ROADMAP.md`, `HISTORY.md` | The plan, open decisions, the core's and the organization's TODO list; the record of phases 0-9. |

- Every built-in plugin is a default feature of `crates/cli`, named after
  its builtin; `nur test` also runs `cargo test -p lsdocs
  --no-default-features` (the core without plugins). Tests that need
  plugins are gated on those features.
- Builds use a cache in `.lsdocs-cache/`; pass `--no-cache` when timing
  builds. `lsdocs serve` (`crates/cli/src/serve.rs`) rebuilds through a
  long-lived `lsdocs_host::Session`.
- Snapshots (`crates/cli/tests/snapshots/`): `nur snapshots` after an
  intended change to rendered output; read the new snapshots before
  accepting them.
- `apidoc-rust`'s and `apidoc-clang`'s builtins need a nightly (`nur
  test` installs the one apidoc-rust's `NIGHTLY` names) and libclang at
  run time for the tests that use them.

### Testing gotchas

- End-to-end tests in `crates/cli/tests/` drive the real `lsdocs` binary
  and compare in-process, stdio and pipe builds byte for byte.
- `crates/cli/tests/serve.rs` runs `lsdocs serve --port 0` and reads its
  log for the port. Keep reading a child's stderr until it exits: closing
  the pipe makes the child's next `eprintln!` panic.
