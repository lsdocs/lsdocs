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
                                        tower-style layers, in-process transport
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
      MSRV is 1.85, the minimum for edition 2024.
- [x] Dev tasks are nushell scripts run through `nur` (`nurfile`, `scripts/`).
      `nur ci` runs the local subset of CI.
- [x] CI on Windows, macOS and Linux: fmt, clippy, test, script tests, MSRV
      check, `cargo deny` (licenses and advisories), all via `nur` tasks.
      Written but not yet run on GitHub.
- [x] Dependency-direction check: `nur check-deps` checks each crate's direct
      workspace dependencies against an allowlist in `scripts/check-deps.nu`.
- [ ] License files (decide MIT, Apache-2.0 or dual). Until then every crate is
      `publish = false`.

**Exit:** `cargo test --workspace` passes in CI on all three operating systems.

## Phase 1: Core types

**Goal:** stable, serializable data types and a published schema.

### `doctree`
- [ ] Core node kinds: Document, Section, Heading, Paragraph, List/ListItem,
      CodeBlock, BlockQuote, Table, Image, Link, and inline text/emphasis/code.
- [ ] `Custom { kind, attrs, children }` extension node.
- [ ] `Span { file, start, end }` on every node.
- [ ] Document metadata (front matter) as a generic value map.
- [ ] Decision: owned nested tree (start here) vs. arena + `NodeId`.

### `config`
- [ ] Site config: input dirs, output dir, base URL, theme, plugin list.
- [ ] Plugin manifest: `command`/`args`, or `runtime = "python" | "node"` +
      module/entry, transport, phase, claimed globs and node kinds.
- [ ] Layered loading (file → env → CLI) and validation with good error messages.

### `protocol`
- [ ] Lifecycle: `initialize`, `initialized`, `shutdown`, `exit`.
- [ ] Phases: `parse`, `transform/collect`, `transform/apply`, `render`.
- [ ] Notifications: `diagnostics`, `progress`, `log`, `$/cancel`.
- [ ] Capabilities: phases, node-kind selectors, `maxConcurrency`, protocol
      version.
- [ ] `Diagnostic` modeled on LSP (severity, span, message, code).

### Serialization rules (msgpack)
- [ ] Structs always encode as **maps**: `rmp_serde::to_vec_named` or
      `with_struct_map()`. Never positional arrays.
- [ ] Externally or adjacently tagged enums for node kinds; avoid internally
      tagged ones on hot paths.
- [ ] `serde_bytes` for all binary payloads.
- [ ] No msgpack `ext` types.

### Schema and compatibility
- [ ] `schemars` export to `schema/`. CI fails if the generated schema differs
      from the checked-in copy.
- [ ] Golden msgpack fixtures per message type, decoded by a small Python
      script in CI (catches struct-as-array regressions).

**Exit:** every protocol type round-trips through msgpack, and the fixtures
decode correctly from Python.

## Phase 2: RPC layer

**Goal:** a symmetric, transport-agnostic RPC peer.

- [ ] Decision: plain **msgpack-rpc** envelopes (`[0,id,method,params]`,
      `[1,id,err,result]`, `[2,method,params]`), which existing client
      libraries can speak, vs. a custom envelope. Leaning msgpack-rpc.
- [ ] Framing: u32 length prefix with a maximum message size (optionally
      switchable to bare msgpack for strict msgpack-rpc compatibility).
- [ ] `Peer`: bidirectional requests and notifications, id allocation,
      cancellation, graceful shutdown.
- [ ] tower-style layers: tracing, concurrency limit, timeouts.
- [ ] Transports: stdio, named pipe / unix socket (path passed via env), and
      in-process (passes typed values directly, no serialization).
- [ ] Stdout protection: duplicate the real stdout handle for RPC and redirect
      fd 1 to stderr (`DuplicateHandle` on Windows, `dup`/`dup2` elsewhere).
- [ ] `--trace-rpc`: log every frame decoded as JSON.
- [ ] `spec/`: a first draft of the wire protocol document.

**Exit:** an echo peer passes the same integration test suite over stdio, a
pipe/socket and in-process.

## Phase 3: Plugin SDK and minimal host

**Goal:** an end-to-end build with stub plugins.

### `plugin-sdk`
- [ ] `Parser`, `Transformer`, `Renderer` traits; capabilities derived from
      what a plugin implements.
- [ ] `serve(plugin)` handles the handshake, dispatch and shutdown.
- [ ] `ctx.log()`, `ctx.diagnostic()`, and host callbacks (e.g. reading the
      collected index).

### `host`
- [ ] Plugin registry from config; spawn, handshake, version check, restart or
      fail policy.
- [ ] Sequential pipeline: discover → parse → collect → apply → render → write.
- [ ] Routing: send only the subtrees whose node kinds a transformer claims.
- [ ] Merge collected data between the `collect` and `apply` phases.
- [ ] Report diagnostics with source spans.

### `cli`
- [ ] `lsdocs build`, `lsdocs init`, with `--config` and `--trace-rpc` flags.

**Exit:** `lsdocs build` runs a stub parser, a transformer and a renderer, both
in-process and out-of-process, with identical output.

## Phase 4: First real site

**Goal:** Markdown in, HTML out.

- [ ] `parser-markdown`: wraps `comrak` or `pulldown-cmark`, maps to doctree,
      front matter, accurate spans.
- [ ] `render-html`: `minijinja` templates, a default theme, asset copying.
- [ ] Each plugin ships as a lib (in-process) and an optional bin
      (out-of-process).
- [ ] Snapshot tests (`insta`) on a sample site.

**Exit:** a multi-page sample docs site builds and renders correctly.

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
| msgpack-rpc envelope vs. custom envelope | msgpack-rpc | Phase 2 |
| Framing: length-prefixed vs. bare msgpack | length prefix, bare optional | Phase 2 |
| Tree representation: nested vs. arena | nested first | Phase 1 |
| Markdown backend: `comrak` vs. `pulldown-cmark` | spike both | Phase 4 |
| Templating engine | `minijinja` | Phase 4 |
| Transformers: whole tree vs. patches | whole tree, patches later | Phase 6 |
| License | — | Phase 0 |
