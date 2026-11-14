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
decode correctly outside Rust (nushell now, Python in Phase 5). Done.

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
pipe/socket and in-memory. Done on Windows and Linux (WSL Ubuntu); macOS not
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
      `progress`, `diagnostic`. Host callbacks (plugin -> host requests) are
      not needed yet, since `transform/apply` already carries the collected
      data; add them when a plugin needs one.
- [x] Typed in-process path: the host calls a builtin `Plugin` directly, with
      no serialization (moved from Phase 2).

### `host`
- [x] Plugin startup from config: builtins in-process; `command`, Python
      (`python -m`) and Node runtimes as child processes over stdio or a
      pipe; handshake with timeouts and protocol version check. Shutdown is
      `shutdown` -> `exit` -> wait, killing a plugin that does not exit. A
      plugin that fails to start fails the build (no restart policy yet).
- [x] Sequential pipeline: discover -> parse -> collect -> apply -> render ->
      write. Files no parser claims are copied as assets; hidden files are
      skipped; `files` globs limit what a plugin sees.
- [x] Routing: a transformer only receives documents containing a node kind
      it selects. Sending just the matching subtrees moved to Phase 7,
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
in-process, over stdio and over a pipe, with byte-identical output. Done on
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
- [x] Links between documents are rewritten (`guide.md#x` -> `guide.html#x`).
      Checking that targets exist is Phase 6 link validation.
- [x] `examples/sample-site` (`nur sample`) with `insta` snapshots of every
      page (`nur snapshots` to re-record).
- [x] MSRV raised from 1.85 to 1.89 for `serde-saphyr` 2.x; `if let` chains
      are now available.

**Exit:** a multi-page sample docs site builds and renders correctly. Done.
Snapshots match on Windows and Linux (WSL). The 1.89 MSRV build itself has
not been checked locally (toolchain not installed).

## Phase 5: Python and Node.js bindings

**Goal:** plugin authors in Python and Node never deal with framing, the
handshake or dispatch. The two APIs follow each language's idioms rather than
matching exactly.

### `sdk-dynamic` (done)
- [x] `DynamicPlugin`: value-based handlers (`Fn(Value, Context) -> Future<Result<Value>>`)
      turned into an SDK `Plugin`, so capabilities, dispatch, cancellation
      and shutdown all stay in Rust.
- [x] Shorthand results for handlers: a bare document, any collect value,
      or a render string; `source` and `extension` filled in.
- [x] All protocol behavior lives here or below, never in a binding.

### 5a: Python (`bindings/python`) (done)
- [x] pyo3 0.30 + maturin, one `abi3` wheel for Python >= 3.10.
- [x] Decorator API: `@plugin.parse(extensions=...)`, `@plugin.collect`,
      `@plugin.transform(nodes=...)`, `@plugin.render(formats=...)`,
      `plugin.serve()`.
- [x] Sync handlers, run on a blocking thread holding the GIL while the
      protocol runs with the GIL released. Exceptions reach the host with
      their traceback.
- [x] `async def` handlers: coroutines run on one persistent asyncio loop
      (a daemon thread), awaited from the handler thread with the GIL
      released. Loop-bound objects survive across calls, and with
      `max_concurrency > 1` async handlers overlap. Pure Python; no
      `pyo3-async-runtimes` needed.
- [x] Params passed as dicts (`pythonize`); results converted back,
      including `bytes` for render output.
- [x] `max_concurrency = 1` by default (GIL); revisit for free-threaded
      builds.
- [x] `serve()` redirects `sys.stdout` to stderr (on top of the fd-level
      protection in `rpc`).
- [x] `Context`: `site`, `options`, `log`/`info`/`warn`/`debug`, `progress`,
      `diagnostic`.
- [x] `lsdocs_plugin.testing`: an in-process fake host for pytest.
- [x] Hand-written `_native.pyi` stubs and `py.typed`. pyo3's own stub
      generation is not mature enough yet; revisit later.
- [ ] `TypedDict`s for params, results and nodes generated from `schema/`.
- [x] `examples/transform_toc.py`: a Python port of `transform-toc`. Over
      stdio, over a pipe and via `runtime = "python"`, it renders the sample
      site byte-identically to the Rust plugin.
- [x] Every file in `fixtures/` decodes with Python's `msgpack` to its JSON
      twin.
- [x] `nur test python` (in `nur ci`): builds the extension with maturin and
      runs pytest, with a venv per platform.
- [x] Decision: a private uv workspace at the repo root (`pyproject.toml`,
      `package = false`) with `bindings/python` as a member, so there is one
      lockfile and one venv for all Python in the repo.

### 5b: Node (`bindings/node`) (done)
- [x] napi-rs 3, `definePlugin({ name, parse, collect, transform, render }).serve()`.
- [x] Handlers are weak threadsafe functions (they don't keep the event loop
      alive); sync or `async` handlers both work, and returned Promises are
      awaited in Rust. Values cross as JSON strings.
- [x] `serve()` returns a Promise that resolves on shutdown.
- [x] `serve()` redirects `process.stdout.write` to stderr.
- [x] `Context` (`site`, `options`, logging, progress, diagnostics). Events
      reach the host when the handler finishes, not as they happen.
- [x] `@lsdocs/plugin/testing`: an in-process fake host, used with the
      built-in `node:test` runner (no vitest dependency).
- [x] Hand-written `index.d.ts` (nodes, params, results, context); the example
      is type-checked against it with `tsc --checkJs`.
- [ ] `.d.ts` types generated from `schema/`.
- [x] `examples/transform-toc.js`: a JS port of `transform-toc`, byte-identical
      to the Rust plugin over stdio, a pipe and `runtime = "node"`.
- [x] Every file in `fixtures/` decodes with `@msgpack/msgpack`.
- [x] `nur test node` (in `nur ci`) builds the addon with plain cargo, since
      napi's CLI can't write atomically on WSL's `/mnt/c`. `nur gen node`
      regenerates the committed loader (`native.js` / `native.d.ts`).
      Node.js comes from `PATH`, or from fnm.

### 5c: Packaging
- [ ] Release CI: wheels and per-platform npm packages for win/mac/linux on
      x64 and arm64, plus linux-musl.
- [ ] Binding packages declare their protocol version; the host rejects a
      mismatch with a clear error. (The host check exists since Phase 3; the
      packages report `PROTOCOL_VERSION`.)
- [ ] Type-check generated `.pyi` / `.d.ts` types against the fixtures.
- [ ] Free-threaded Python wheels. The `abi3` wheel cannot load on
      free-threaded builds (3.13t/3.14t), so these need one version-specific
      wheel each (`cp313t`, `cp314t`); pyo3 0.30 falls back to that
      automatically when built against a free-threaded interpreter. From
      Python 3.15, a single `abi3t` wheel (pyo3's `abi3t-py3*` features) can
      cover them. The code is already free-threading-ready: pyo3 modules
      default to `gil_used = false`, and nothing relies on the GIL for
      correctness. Not yet run on a free-threaded build. Consider raising the
      default `max_concurrency` there (`sys._is_gil_enabled()`), keeping 1
      since parallel sync handlers need thread-safe plugin code.

**Exit:** the Python and TS `transform-toc` plugins pass the shared test suite
and install from built wheels and npm tarballs. Done for both, built and
installed locally; release wheels and npm packages are 5c.

## Phase 6: Transformers and cross-document features

Split into three branches: 6a links, 6b navigation and sitemap, 6c syntax
highlighting.

### 6a: Links (done)
- [x] `SiteInfo.files`: the host discovers files before starting plugins
      and sends every source file (documents and assets) in `initialize`,
      so transformers can check links to images and other assets.
- [x] `transform-toc` nests deeper headings under the one before them (the
      Python and JS ports too).
- [x] `transform-links` (new builtin), using `transform/collect` ->
      `transform/apply`:
  - link and image checking: `broken-link` and `broken-anchor` warnings
    with spans; external URLs are not checked;
  - cross-references: a link with no text (`[](guide.md#setup)`) takes the
    title of the heading or page it points to;
  - backlinks (`backlinks = true`): a `custom:backlinks` list of the pages
    linking here.
- [x] `render-html`: `#` anchor links on headings (`heading_anchors`,
      default on). Link rewriting was done in Phase 4.
- [x] `plugin_sdk::testing`: a context that drops events, and
      `run_transform`, which runs collect then apply like the host.

### 6b: Navigation and sitemap (done)
- [x] `transform-nav` (new builtin): collects titles and `nav_title` /
      `nav_order` / `nav_exclude` front matter, then gives every document a
      `nav` metadata entry: a tree following the source directories (a
      directory's `index` page titles its section), with the current page
      marked and `prev`/`next` links. Each page carries the whole tree, so
      metadata grows with the site; revisit in Phase 7 if it matters.
- [x] `render/site` (new request): sent once after rendering to a renderer
      whose capability says `site: true`, with every rendered page (source,
      output path, metadata). It returns extra files; the host rejects paths
      that leave the output directory or replace a rendered page.
- [x] `render-html`: writes `sitemap.xml` through `render/site` when
      `base_url` is absolute (decided: part of the HTML renderer, not a
      separate phase or host feature); front matter `sitemap: false` leaves
      a page out. The default theme shows `page.nav` as a sidebar with
      previous/next links, and templates get a `link` filter that maps
      source links to outputs.
- [x] Shared helpers: `doctree::plain_text`, `Document::title` and
      `plugin_sdk::relative_path` replace copies in the plugins.
- `render/site` support in `sdk-dynamic` and the bindings moved to
  Phase 7.

### 6c: Syntax highlighting (done)
- [x] `transform-highlight` (new builtin; decided: a transformer plugin, so
      highlighting can be swapped out). It uses `syntect` (pure-Rust
      `fancy-regex` backend) with `bat`'s syntaxes from `two-face`, since
      `syntect`'s own set lacks TOML and TypeScript. Each highlighted block
      becomes a `custom:highlight` node whose `attrs.html` holds the HTML
      and whose child is the original `code_block`, so other renderers can
      fall back to it. Tokens carry `hl-`-prefixed scope classes; the
      default theme colors them for light and dark. Options: `languages`,
      `aliases`.
- [x] `render-html` writes `custom:highlight` nodes' HTML as is.
- Note: `syntect` 5 depends on `bincode` 1.3, which has an "unmaintained"
  RustSec advisory; `nur deny` may need an exception until `syntect`
  moves off it.

Patch-based `transform` responses moved to Phase 7 (decided): they are a
performance feature, to be decided with the benchmarks there.

### 6d: Parser/transformer contract

**Goal:** what a parser hands to transformers is the same shape whatever the
source format, so a reStructuredText parser (Phase 8) or a third-party
parser works with every first-party transformer and renderer unchanged.
Today the contract is implicit and Markdown-shaped: flat headings, ids
slugged by the Markdown parser, cross-document links as `.md` URLs, and no
nodes for admonitions, footnotes and the like.

Decided:

- **Nested sections are canonical.** Parsers wrap content in `section`
  nodes: the section's title heading first, its subsections nested inside.
  The anchor id lives on the `section`; `heading.id` remains only for
  headings outside the section tree (e.g. inside a block quote).
- **The host assigns generated ids.** Parsers set only explicit ids
  (`{#id}`, RST labels). After parsing, the host gives every section (and
  stray heading) without an id a slug of its title, unique within the
  document, the same way for every format and plugin language. The slug
  rules move from `parser-markdown` to the host and into `spec/`.
- **Common constructs become core node kinds:** `admonition` (`kind`,
  optional title), definition lists, footnote references and definitions,
  `math` (inline or display), `figure` (content plus caption), and
  `anchor` (a zero-width link target, e.g. an RST label before a
  paragraph). Format-specific extras stay `custom:*`.
- **Links carry structured targets.** `Link` gains an optional `target`:
  a document (source path) and/or an anchor, or an unresolved label. The
  host fills `target.document` for any relative `url` that resolves to a
  parsed document, so parsers need no path logic; RST's `:doc:`/`:ref:`
  set it directly. `transform-links` resolves labels site-wide; renderers
  map a target's document to its output path instead of guessing from the
  `.md` extension. `url` stays for external links.

Steps (one branch each, in order):

- [x] 6d-1 Doctree and protocol: `section` as described, the new node
      kinds, `Link.target`; schemas, fixtures, `spec/` and the bindings'
      type declarations (`index.d.ts`) updated.
- [x] 6d-2 Host normalization after parse: slug ids (moved from the
      parser), `Link.target` for relative links to documents, a warning for
      duplicate explicit ids.
- [x] 6d-3 `parser-markdown`: build sections from heading levels; GitHub
      alerts (`> [!NOTE]`) as admonitions; footnotes, definition lists and
      math via `pulldown-cmark` options; no more slugging.
- [x] 6d-4 Transformers: `transform-toc` walks sections (and its Python
      and JS ports), `transform-links` uses targets, section ids and
      `anchor` nodes and resolves labels, `transform-nav` takes titles from
      sections.
- [x] 6d-5 `render-html`: `<section id>`, anchor links on section titles,
      the new node kinds, output URLs from link targets; snapshots
      re-recorded and reviewed.
- 6d-3 to 6d-5 landed as one commit: once the parser emits sections, the
  transformers and the renderer must follow for the build to stay correct.
  Also in it: `transform-links` resolves `label` targets (`broken-label`,
  `ambiguous-label`); `transform-nav` items carry their `source`, and
  `render-html` templates get a `page_url` filter that maps a source path
  to its output; display math renders as a `<span>`, since it can sit
  inside a paragraph.
- Later: render footnote definitions together at the end of the page
  rather than where they appear in the source.

## Phase 7: Developer experience and performance

- [ ] Parallel parse and render across documents, respecting each plugin's
      `maxConcurrency`.
- [ ] Content-hash cache with per-plugin version keys.
- [ ] Incremental rebuilds driven by a dependency graph, including
      cross-document dependencies from the collect phase.
- [ ] `lsdocs serve`: file watching, dev server, live reload.
- [ ] Benchmarks: in-process vs. out-of-process cost, tree size vs. latency.
- [ ] `render/site` in `sdk-dynamic` and the Python/Node bindings, so
      renderers in those languages can write site-wide files (moved from
      Phase 6b).
- [ ] Optional patch-based `transform` responses, and sending transformers
      only the subtrees they select, if the benchmarks show tree size
      matters (moved from Phase 6).

## Phase 8: Toward 1.0

- [ ] Freeze protocol v1 and write the compatibility policy.
- [ ] Finalize `spec/` so authors in other languages (Go, Lua, ...) can implement
      plugins without bindings.
- [ ] Plugin authoring guide and docs site, built with lsdocs itself.
- [ ] First-party reStructuredText parser (`parser-rst`), on top of the
      Phase 6d contract.
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
| Transformers: whole tree vs. patches | whole tree; patches decided with benchmarks | Phase 7 |
| ~~License~~ | **Decided:** `Apache-2.0 OR MIT` | Phase 0 |
| ~~Parser-neutral cross-references~~ | **Decided:** structured link targets; see Phase 6d | Phase 6d |
| ~~Canonical document shape~~ | **Decided:** nested sections, host-generated ids, more core node kinds; see Phase 6d | Phase 6d |

### Parser-neutral cross-references (background for 6d)

Today a link between documents is a plain `url` such as `guide.md#setup`.
`transform-links` resolves those paths generically (any extension), but:

- `render-html` turns a link into a page link only if it ends in `.md` or
  `.markdown`, so `guide.rst` would not be rewritten.
- reStructuredText references are not paths: `:doc:` names a document
  without its extension, and `:ref:` names a label (`.. _label:`) defined
  anywhere in the site. No URL captures those.

Decided (Phase 6d): give `Link` an optional structured `target` (document
source path, anchor, or an unresolved label), filled by the host for
relative links to documents and by parsers for format-specific references. `transform-links` resolves labels and checks
targets using what it collects; renderers turn a resolved target into an
output URL from the source path, instead of guessing by file extension.
Plain `url`s stay for external links and as the fallback.
