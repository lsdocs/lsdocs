# lsdocs Roadmap

lsdocs is a static-site generator written in Rust. Its extension model borrows
the Language Server Protocol's design: parsers, transformers and renderers are
**plugins** that talk to a central **host** over an RPC protocol. A plugin can
be written in any language. First-party plugins can run in-process at no IPC
cost, and Python and Node.js plugin authors get bindings that hide the protocol
plumbing.

## Guiding principles

- **One contract, many transports.** In-process and out-of-process plugins
  implement the same traits and go through the same `Peer` interface.
- **The wire format is the public API.** JSON Schema is the source of truth for
  every type that crosses a process boundary. Bindings and non-Rust clients are
  generated from it.
- **Own the protocol stack.** `lsp-types`, `tower-lsp` and similar crates are
  references only. They're editor-specific and pin outdated transitive
  dependencies.
- **Dependencies only point downward.** Data crates know nothing about RPC, and
  plugins never depend on the host.

## Workspace layout

```
lsdocs/
  crates/
    doctree/        lsdocs-doctree      AST: nodes, spans, metadata (serde only)
    config/         lsdocs-config       site config + plugin manifests (serde only)
    protocol/       lsdocs-protocol     message & capability types (serde only)
    rpc/            lsdocs-rpc          msgpack envelope, framing, codec, Peer,
                                        stdio/pipe/in-memory transports
    plugin-sdk/     lsdocs-plugin-sdk   Parser/Transformer/Renderer traits + serve()
    sdk-dynamic/    lsdocs-sdk-dynamic  value-based handler adapter for bindings
    host/           lsdocs-host         registry, supervision, pipeline, cache
    cli/            lsdocs              the `lsdocs` binary
  plugins/
    parser-markdown/
    transform-toc/
    render-html/
  bindings/
    python/         pyo3 + maturin  -> PyPI `lsdocs-plugin` (import `lsdocs_plugin`)
    node/           napi-rs         -> npm `@lsdocs/plugin`
  schema/           generated JSON Schemas (checked in, diffed in CI)
  spec/             wire protocol specification
```

```
doctree    config
    \        /
    protocol
        |
       rpc
     /     \
plugin-sdk  host
   |   \      |
   |  sdk-dynamic   cli
   |     /    \
plugins  python  node
```

Package names are working names; check them on crates.io, PyPI and npm before
publishing anything.

---

## Phase 0: Skeleton

**Goal:** an empty workspace that builds, tests and enforces the dependency
graph.

- [x] Cargo workspace with all crates stubbed, shared `[workspace.package]` and
      `[workspace.dependencies]`.
- [x] `rust-toolchain.toml`, `rustfmt.toml`, clippy config (workspace lints).
      MSRV is 1.89 (raised from 1.85, the edition 2024 minimum, in Phase 4
      for `serde-saphyr` 2.x).
- [x] Dev tasks are nushell scripts run through `nur` (`nurfile`, `scripts/`).
      `nur ci` runs the local subset of CI.
- [x] CI on Windows, macOS and Linux: fmt, clippy, test, script tests, MSRV
      check, `cargo deny` (licenses and advisories), all via `nur` tasks.
      Written but not yet run on GitHub.
- [x] Dependency-direction check: `nur check-deps` checks each crate's direct
      workspace dependencies against an allowlist in `scripts/check-deps.nu`.
- [x] License: `Apache-2.0 OR MIT` (`LICENSE-APACHE`, `LICENSE-MIT`). Crates
      stay `publish = false` until package names are checked.

**Exit:** `cargo test --workspace` passes in CI on all three operating systems.

## Phase 1: Core types

**Goal:** stable, serializable data types and a published schema.

### `doctree`
- [x] Core node kinds: Section, Heading, Paragraph, List/ListItem,
      BlockQuote, CodeBlock, Table/TableRow/TableCell, ThematicBreak, Raw,
      Text, Emphasis, Strong, InlineCode, Link, Image, LineBreak.
- [x] `Custom { kind, attrs, children }` extension node, selected as
      `custom:<kind>`.
- [x] `Span { start, end }` (byte offsets) on every node. The file lives once
      on `Document::source` rather than on every span.
- [x] Document metadata (front matter) as a map of JSON-like values.
- [x] Decision: owned nested tree. Revisit arena + `NodeId` only if patching
      (Phase 6) needs it.

### `config`
- [x] `lsdocs.toml`: `[site]` (title, base URL, theme), `[build]` (source and
      output dirs), `[[plugins]]`. Unknown keys are rejected.
- [x] Plugin entry: exactly one of `builtin`, `command`, or `runtime` +
      `module`/`entry`; `transport`, `files` globs, free-form `options`.
      Phases and node kinds come from the plugin's capabilities instead.
- [x] Validation reports every problem at once, with its location. Overrides
      layer env (`LSDOCS_*`) then CLI on top. The crate does no I/O.

### `protocol`
- [x] Lifecycle: `initialize`, `initialized`, `shutdown`, `exit`.
- [x] Phases: `parse`, `transform/collect`, `transform/apply`, `render`.
- [x] Notifications: `diagnostics`, `progress`, `log`, `$/cancel`.
- [x] Capabilities: per-phase (parse extensions, transform node selectors and
      `collect`, render formats), `max_concurrency`; `protocol_version` in the
      handshake (0 = unstable until v1).
- [x] `Diagnostic` modeled on LSP (severity, message, source, span, code).
- [x] `Request`/`Notification` traits tie each method name to its types.

### Serialization rules (msgpack)
- [x] Structs always encode as **maps**: `protocol::msgpack::encode` wraps
      `rmp_serde::to_vec_named`. Never positional arrays.
- [x] Node kinds are **internally tagged** (`{"type": "heading", ...}`), a
      deliberate change from the original plan: it is the most natural shape
      for Python dicts and TS discriminated unions. Decoding buffers each
      node; revisit with a custom deserializer if Phase 7 benchmarks show it
      matters.
- [x] `snake_case` field names everywhere; optional fields omitted when
      empty; unknown fields ignored (except in `lsdocs.toml`).
- [x] `serde_bytes` for binary payloads (render output).
- [x] No msgpack `ext` types.

### Schema and compatibility
- [x] `tools/gen` (`nur gen`) writes JSON Schemas to `schema/` for the
      document, `lsdocs.toml` and every message. A test fails if the
      checked-in files are stale.
- [x] Golden fixtures in `fixtures/`: each example value as `.msgpack` plus a
      `.json` twin. `nur test fixtures` decodes the msgpack with nushell's own
      decoder and compares, catching struct-as-array regressions. A Python
      decoder check moves to Phase 5, alongside the bindings.

**Exit:** every protocol type round-trips through msgpack, and the fixtures
decode correctly outside Rust (nushell now, Python in Phase 5). ✅

## Phase 2: RPC layer

**Goal:** a symmetric, transport-agnostic RPC peer.

- [x] Decision: plain **msgpack-rpc** envelopes (`[0,id,method,params]`,
      `[1,id,err,result]`, `[2,method,params]`). Params are the msgpack-rpc
      argument array: `[]` or `[params]`.
- [x] Framing: u32 big-endian length prefix, 64 MiB default limit. Oversized
      outgoing messages are rejected before sending; an oversized incoming
      frame ends the connection. Bare (unframed) msgpack is not implemented;
      add it only if a real msgpack-rpc client needs it.
- [x] `Peer` + `Router` + `Connection`: bidirectional requests and
      notifications, typed handlers, out-of-order responses, `$/cancel` both
      ways (dropping or timing out a request cancels it), graceful shutdown
      that lets in-flight handlers answer.
- [x] Tracing, concurrency limit and request timeouts as connection
      `Options` rather than tower layers, which avoids the `tower` dependency.
- [x] Transports: stdio, named pipe / unix socket (`LSDOCS_PIPE`), and an
      in-memory pair. Typed in-process plugins with no serialization move to
      Phase 3, where the SDK traits they call exist.
- [x] Stdout protection (`transport::protect_stdout`): duplicate the real
      stdout and point stdout at stderr (`DuplicateHandle` + `SetStdHandle` +
      CRT `_dup2` on Windows, `fcntl`/`dup2` elsewhere).
- [x] Wire tracing: every frame is logged as JSON on the `lsdocs_rpc::wire`
      tracing target. The CLI's `--trace-rpc` flag will enable it (Phase 3).
- [x] `spec/wire.md`: first draft of the wire protocol.

- [x] stdio reads stdin on a detached thread, so a plugin that ends its
      connection exits even while the host still holds its stdin open
      (tokio's own stdin cannot be cancelled and hung shutdown).

**Exit:** an echo peer passes the same integration test suite over stdio, a
pipe/socket and in-memory. ✅ on Windows and Linux (WSL Ubuntu); macOS not
yet run.

## Phase 3: Plugin SDK and minimal host

**Goal:** an end-to-end build with stub plugins.

### `plugin-sdk`
- [x] `Parser`, `Transformer`, `Renderer` traits (implementable with plain
      `async fn`); a `Plugin` bundles any of them, and capabilities follow
      from what it has.
- [x] `main(plugin)` / `serve(plugin)` handle the handshake, dispatch and
      shutdown over stdio or `LSDOCS_PIPE`. Phases a plugin lacks answer
      "method not found"; requests before `initialize` are refused.
- [x] `Context`: `site()`, `options()`, `log`/`info`/`warn`/`debug`,
      `progress`, `diagnostic`. Host callbacks (plugin → host requests) are
      not needed yet, since `transform/apply` already carries the collected
      data; add them when a plugin needs one.
- [x] Typed in-process path: the host calls a builtin `Plugin` directly, with
      no serialization (moved from Phase 2).

### `host`
- [x] Plugin startup from config: builtins in-process; `command`, Python
      (`python -m`) and Node runtimes as child processes over stdio or a
      pipe; handshake with timeouts and protocol version check. Shutdown is
      `shutdown` → `exit` → wait, killing a plugin that does not exit. A
      plugin that fails to start fails the build (no restart policy yet).
- [x] Sequential pipeline: discover → parse → collect → apply → render →
      write. Files no parser claims are copied as assets; hidden files are
      skipped; `files` globs limit what a plugin sees.
- [x] Routing: a transformer only receives documents containing a node kind
      it selects. Sending just the matching subtrees moves to Phase 6,
      alongside patch-based responses.
- [x] Collected data is merged per document (`{source: data}`) between the
      `collect` and `apply` passes.
- [x] Diagnostics are reported as `severity[code]: file:line:col: message`;
      any error diagnostic makes `lsdocs build` exit non-zero.

### `cli`
- [x] `lsdocs build` (`--config`, `--source-dir`, `--output-dir`,
      `--base-url`), `lsdocs init`, global `-v`/`-vv` and `--trace-rpc`.
      `LSDOCS_LOG` overrides log filtering.
- [x] Hidden `lsdocs plugin <name>` serves a builtin out-of-process, so every
      first-party plugin has an out-of-process form without its own binary.

### First-party plugins (minimal versions)
- [x] `parser-markdown`: front matter, headings, fenced code, paragraphs.
      Inline markup is plain text until Phase 4.
- [x] `transform-toc`: collects headings, inserts a `custom:toc` list of
      links; `depth` option.
- [x] `render-html`: standalone escaped HTML pages covering every node kind.

**Exit:** `lsdocs build` runs a parser, a transformer and a renderer
in-process, over stdio and over a pipe, with byte-identical output. ✅ on
Windows and Linux (WSL).

## Phase 4: First real site

**Goal:** Markdown in, HTML out.

- [x] `parser-markdown` on `pulldown-cmark`: CommonMark plus tables and task
      lists, every node with its byte span. YAML (`---`) or TOML (`+++`) front
      matter, with invalid front matter reported as a warning. Heading ids
      from `{#id}` or a unique slug. Raw HTML becomes `raw` nodes.
- [x] `render-html`: a `minijinja` page template with a built-in default theme
      (light/dark, no external assets), or a custom one via the `template`
      option, resolved against the site root. (Asset copying is done by the
      host since Phase 3.)
- [x] `SiteInfo.root` added to the protocol, so plugins can resolve paths
      from their options even when running in-process.
- [x] Each plugin ships as a lib (in-process) and runs out-of-process via
      `lsdocs plugin <name>` (done in Phase 3, instead of one bin each).
- [x] Links between documents are rewritten (`guide.md#x` → `guide.html#x`).
      Checking that targets exist is Phase 6 link validation.
- [x] `examples/sample-site` (`nur sample`) with `insta` snapshots of every
      page (`nur snapshots` to re-record).
- [x] MSRV raised from 1.85 to 1.89 for `serde-saphyr` 2.x; `if let` chains
      are now available.

**Exit:** a multi-page sample docs site builds and renders correctly. ✅
Snapshots match on Windows and Linux (WSL). The 1.89 MSRV build itself has
not been checked locally (toolchain not installed).

## Phase 5: Python and Node.js bindings

**Goal:** plugin authors in Python and Node never deal with framing, the
handshake or dispatch. The two APIs follow each language's idioms rather than
matching exactly.

### `sdk-dynamic`
- [ ] Handler table of `Fn(Value) -> Future<Result<Value>>`.
- [ ] Capability builder; owns the tokio runtime and transport setup.
- [ ] All protocol behavior lives here, never in a binding.

### Python (`bindings/python`)
- [ ] pyo3 + maturin, `abi3` wheels (e.g. `abi3-py310`).
- [ ] Decorator API: `@plugin.transform(nodes=[...])`, `plugin.serve()`.
- [ ] Sync handlers first, then `async def` via `pyo3-async-runtimes`.
- [ ] Trees converted to/from dicts with `pythonize`.
- [ ] Report `maxConcurrency = 1` under the GIL; revisit for free-threaded
      builds.
- [ ] Redirect `sys.stdout` to stderr in `serve()`.
- [ ] `.pyi` stubs with `TypedDict`s generated from `schema/`.

### Node (`bindings/node`)
- [ ] napi-rs, `definePlugin({...}).serve()` API.
- [ ] Handlers are called via `ThreadsafeFunction`; returned Promises are
      awaited in Rust.
- [ ] `serve()` returns a Promise that resolves on shutdown.
- [ ] Redirect `process.stdout.write` to stderr.
- [ ] `.d.ts` doctree types generated from `schema/`.

### Both
- [ ] Testing harness: an in-process fake host (`lsdocs_plugin.testing.call`
      and the Node equivalent) for pytest and vitest.
- [ ] A port of `transform-toc` in each language, producing output
      byte-identical to the Rust plugin.
- [ ] Release CI: wheels and per-platform npm packages for win/mac/linux on x64
      and arm64, plus linux-musl.
- [ ] Binding packages declare their protocol version; the host rejects a
      mismatch with a clear error.
- [ ] Decode every file in `fixtures/` from Python (the `msgpack` package)
      and Node (`@msgpack/msgpack`), compared against the JSON twins.
- [ ] Generate `.pyi` and `.d.ts` types from `schema/` and type-check them
      against the fixtures.

**Exit:** the Python and TS `transform-toc` plugins pass the shared test suite
and install from built wheels and npm tarballs.

## Phase 6: Transformers and cross-document features

- [ ] Table of contents, heading anchors, link rewriting and validation.
- [ ] Syntax highlighting (e.g. `syntect` or tree-sitter based).
- [ ] Cross-references and backlinks via `transform/collect` →
      `transform/apply`.
- [ ] Site-wide navigation and sitemap.
- [ ] Optional patch-based `transform` responses for large trees.

## Phase 7: Developer experience and performance

- [ ] Parallel parse and render across documents, respecting each plugin's
      `maxConcurrency`.
- [ ] Content-hash cache with per-plugin version keys.
- [ ] Incremental rebuilds driven by a dependency graph, including
      cross-document dependencies from the collect phase.
- [ ] `lsdocs serve`: file watching, dev server, live reload.
- [ ] Benchmarks: in-process vs. out-of-process cost, tree size vs. latency.

## Phase 8: Toward 1.0

- [ ] Freeze protocol v1 and write the compatibility policy.
- [ ] Finalize `spec/` so authors in other languages (Go, Lua, …) can implement
      plugins without bindings.
- [ ] Plugin authoring guide and docs site, built with lsdocs itself.
- [ ] Publish crates, wheels and npm packages.

---

## Open decisions

| Decision | Leaning | Decide by |
|---|---|---|
| ~~Envelope~~ | **Decided:** msgpack-rpc | Phase 2 |
| ~~Framing~~ | **Decided:** u32 length prefix; bare msgpack only if needed | Phase 2 |
| ~~Tree representation~~ | **Decided:** nested; revisit for patches | Phase 1 |
| ~~Markdown backend~~ | **Decided:** `pulldown-cmark` (byte-offset spans map directly to `Span`) | Phase 4 |
| ~~Templating engine~~ | **Decided:** `minijinja` | Phase 4 |
| Transformers: whole tree vs. patches | whole tree, patches later | Phase 6 |
| ~~License~~ | **Decided:** `Apache-2.0 OR MIT` | Phase 0 |
