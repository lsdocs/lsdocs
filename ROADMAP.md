# lsdocs Roadmap

lsdocs is a static-site generator written in Rust. Its extension model borrows
the Language Server Protocol's design: parsers, transformers and renderers are
**plugins** that talk to a central **host** over an RPC protocol. A plugin can
be written in any language. First-party plugins can run in-process at no IPC
cost, and Python and Node.js plugin authors get bindings that hide the protocol
plumbing.

This file is the plan from here on. What phases 0-9 did, and the decisions
made along the way, are in [HISTORY.md](HISTORY.md). Record new decisions and
deviations here as you go, under the phase they belong to.

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
- **Libraries stand alone.** `lsdocs-rst` and the API documentation framework
  are useful outside lsdocs: they depend on nothing from it, and lsdocs plugins
  are thin adapters over them.
- **Conformance over resemblance.** Where lsdocs implements something with a
  reference implementation (docutils, Sphinx, the zensical theme), it is
  tested against that implementation's output, not against a reading of its
  docs.
- **Errors, not panics.** A bad document, plugin or file is a diagnostic or
  an error, never a crash. Clippy denies `unwrap`, `expect` and `panic!`
  outside tests (decided after Phase 9; see AGENTS.md).

## Where things stand

Phases 0-9 are done (see [HISTORY.md](HISTORY.md)):

| Phase | Result |
|---|---|
| 0-3 | Workspace, data crates (`doctree`, `config`, `protocol`), msgpack-rpc layer, plugin SDK and host. |
| 4 | A real site: `parser-markdown`, `transform-toc`, `render-html`. |
| 5 | Python and Node.js plugin bindings (packaging deferred, 5c). |
| 6 | Links, navigation, sitemap, syntax highlighting; the shared document shape. |
| 7 | Parallel pipeline, build cache, `lsdocs serve`, binding performance. |
| 8 | `lsdocs-rst`, a docutils port checked against docutils (parser, directives, transforms, file access), and `parser-rst`. |
| 9 | Protocol `1.0-rc.0` and its compatibility policy, the finished spec, a plugin written from the spec alone, and the docs site. |

Not done from those phases: packaging and publishing (5c, 9), freezing the
protocol as 1.0 (9), and the items in the [TODO list](#todo-list).

## Workspace layout

Today, plus what the phases below add (marked *planned*):

```
lsdocs/
  crates/
    doctree/  config/  protocol/  rpc/  plugin-sdk/  sdk-dynamic/  host/  cli/
    rst/            lsdocs-rst: the reStructuredText library
    apidoc/         API documentation framework            (planned, Phase 12)
  plugins/
    parser-markdown/  parser-rst/
    transform-toc/  transform-links/  transform-nav/  transform-highlight/
    render-html/    themes: default; zensical              (planned, Phase 13)
    sphinx-addons/  Sphinx's additions to docutils         (planned, Phase 11)
    apidoc-python/  apidoc-typescript/  apidoc-cpp/  apidoc-rust/
                                                            (planned, Phase 12)
  bindings/
    plugin/python/  plugin/node/    the plugin SDK bindings
    rst/python/     rst/node/       lsdocs-rst bindings     (planned, Phase 10)
  docs/  examples/  schema/  fixtures/  spec/  scripts/  tools/
```

Package names are working names; check them on crates.io, PyPI and npm before
publishing anything.

---

## Phase 10: The reStructuredText library, from other languages

**Goal:** `lsdocs-rst` usable from Python and JavaScript, as packages of its
own, not only inside lsdocs. A docutils-compatible parser that is much faster
than docutils is worth having on its own, and the bindings are also how
Python and Node.js plugins (or Sphinx-like tools) can reuse it.

### 10a: Bindings layout
- [x] Move the plugin SDK bindings to `bindings/plugin/python` and
      `bindings/plugin/node`; the rst bindings go to `bindings/rst/python`
      and `bindings/rst/node`. Each is its own package with its own tests.
- [x] Shared tooling: the uv workspace gains the new Python member; an npm
      workspace (or per-package installs) for the Node packages; `nur test
      python` / `nur test node` run every package (or take a package name);
      `nur gen node` handles each addon. Update AGENTS.md and
      `check-deps.nu` (a rule per bindings group).
- [x] Decide the library's public name (it is `lsdocs-rst` until then) for
      the crate and both packages.

Decisions:
- The name stays **`lsdocs-rst`**: the crate `lsdocs-rst`, the PyPI
  package `lsdocs-rst` (module `lsdocs_rst`) and the npm package
  `@lsdocs/rst`, in the same namespace as `lsdocs-plugin` and
  `@lsdocs/plugin`.
- Binding crates are named after their package: `lsdocs-plugin-python`,
  `lsdocs-plugin-node` (renamed from `lsdocs-python`, `lsdocs-node`),
  `lsdocs-rst-python`, `lsdocs-rst-node`.
- Node packages are installed one by one, each with its own lockfile, not
  as an npm workspace: they share no JS dependencies worth hoisting, and
  per-package installs keep each package publishable as it is. The `nur`
  tasks list the packages (`PYTHON_PACKAGES`, `NODE_PACKAGES`).

### 10b: A public API worth binding
- [ ] Settings: one `Settings` type for the docutils settings the parser
      honors (`doctitle`, `report_level`, `pep_references`,
      `rfc_references`, `tab_width`, `line_length_limit`, `smart_quotes`,
      `sectsubtitle_xform`, `toc_backlinks`, `auto_id_prefix`, ...), with
      docutils' defaults; the builder methods stay as shorthands.
- [ ] Stable tree accessors (kinds, attributes, children, spans) and a
      serde representation of the tree, so bindings and tools can exchange
      it as JSON or MessagePack.
- [ ] Writers: pseudo-XML (exists), and docutils' XML writer, checked
      against docutils. HTML is lsdocs' job, not the library's.
- [ ] rustdoc for every public item, with examples; the crate's README.

### 10c: Python package (`bindings/rst/python`)
- [ ] pyo3 + maturin, abi3. `parse(text, **settings)` returns a tree of
      Python objects (kind, attributes, children, span), with `pformat()`
      and `to_dict()`.
- [ ] Directives and roles written in Python, registered on a parser (the
      `Directive` and `Role` traits over Python callables), and a file
      loader callback.
- [ ] Optional: conversion to a real `docutils.nodes.document`, for use
      with docutils writers and Sphinx, if docutils is installed.
- [ ] Tests: the conformance cases run through the package too.

### 10d: Node.js package (`bindings/rst/node`)
- [ ] napi-rs: `parse(text, settings)` returning plain objects, with types
      (`.d.ts`) for the tree and settings; directives, roles and the file
      loader as JS functions.
- [ ] Tests: the conformance cases run through the package too.

**Exit:** both packages parse the conformance cases to docutils' trees, and
a directive written in each language works.

## Phase 11: Sphinx add-ons

**Goal:** a `sphinx-addons` plugin implementing what Sphinx adds on top of
docutils, so Sphinx projects build with lsdocs with few changes. Like
`lsdocs-rst`, checked against Sphinx's own output where possible (its
doctree after reading, from a dev-only harness like `nur gen rst`).

### 11a: Architecture
- [ ] Decide how the add-ons hook in. Directives and roles run while
      parsing, inside `lsdocs-rst`, so they cannot come over the protocol
      from another process. Leaning: a Rust crate of directive and role
      implementations that `parser-rst` registers when the site enables it
      (option `sphinx = true`), plus a transformer for the cross-document
      parts (toctrees, numbered references, glossary terms, the index,
      intersphinx). Move `:ref:` and `:doc:` from `parser-rst` there.
- [ ] Decide where the settings Sphinx keeps in `conf.py` go (`project`,
      `version`, `release`, `numfig`, `primary_domain`, `default_role`,
      `rst_prolog` / `rst_epilog`, `intersphinx_mapping`, ...). Leaning:
      plugin options in `lsdocs.toml`; reading `conf.py` is out of scope.

### 11b: Directives
- [ ] Code: `code-block` / `sourcecode` (`:linenos:`, `:emphasize-lines:`,
      `:caption:`, `:name:`, `:dedent:`, `:force:`), `literalinclude`
      (`:lines:`, `:start-after:`, `:end-before:`, `:pyobject:`, `:diff:`,
      ...), `highlight`.
- [ ] Notes: `versionadded`, `versionchanged`, `deprecated`,
      `versionremoved`, `seealso`, `todo` / `todolist` (sphinx.ext.todo).
- [ ] Content: `only` (by builder tags), `centered`, `hlist`, `glossary`,
      `productionlist`, `rubric` with Sphinx's options, `tabularcolumns`
      (accepted, ignored), `sectionauthor` / `codeauthor`, `acks`.
- [ ] Structure: `toctree` (`:maxdepth:`, `:caption:`, `:hidden:`,
      `:glob:`, `:titlesonly:`, `:numbered:`), feeding `transform-nav`: a
      site whose pages have toctrees gets its navigation from them instead
      of from its directories.
- [ ] `index` directive and role, and a generated index page (needs
      generated documents, 12a).
- [ ] Math: `math` with `:label:` and equation numbering, `:eq:`.

### 11c: Roles
- [ ] Cross-references: `ref`, `doc`, `numref` (with `numfig` numbering),
      `term`, `keyword`, `token`, `option`, `envvar`, `any`, `download`
      (copies the file).
- [ ] Inline semantics: `abbr`, `command`, `dfn`, `file`, `guilabel`, `kbd`,
      `mailheader`, `makevar`, `manpage`, `menuselection`, `mimetype`,
      `newsgroup`, `program`, `regexp`, `samp`. These need inline node kinds
      or a convention for `custom` inlines (see the TODO on inline
      semantics).

### 11d: Domains and intersphinx
- [ ] Object descriptions for the standard domains (`py`, `c`, `cpp`, `js`,
      `rst`, `std`): `.. py:function::` and the like, with their signatures
      parsed, and the matching roles (`:py:func:`, `:cpp:class:`, ...). The
      object model is shared with the API documentation framework (Phase
      12), so hand-written and generated API docs link to each other.
- [ ] Intersphinx: read `objects.inv` inventories (the labels and objects of
      other sites, Sphinx or lsdocs) and resolve references to them; write
      one for the lsdocs site.
- [ ] `extlinks`-style link shortcuts.

**Exit:** a mid-sized Sphinx project (to be picked) builds without errors,
and its pages match Sphinx's doctrees for the supported features.

## Phase 12: API documentation framework

**Goal:** documentation generated from source code for Python, TypeScript,
C/C++ and Rust. A language-neutral framework does the common work; each
language is its own package that depends on it and only extracts.

### 12a: Platform additions
- [ ] **Generated documents:** a new, additive protocol capability that
      lets a plugin add documents that have no source file (an API page per
      module, an index page, a search page). Decide where it runs in the
      pipeline (a "generate" step after parsing, so generated pages are
      normalized, transformed and rendered like any other).
- [ ] **Dependencies:** parsers and generators report the files a result
      depends on (source files read, included files), so the cache and
      `lsdocs serve` invalidate exactly; this replaces turning caching off.

### 12b: The framework (`crates/apidoc`)
- [ ] The model: modules, namespaces and packages; classes, structs,
      interfaces, traits, enums and variants; functions, methods and
      overloads; fields and properties; constants, type aliases, macros.
      Signatures with parameters, defaults, types and generics; visibility,
      inheritance, re-exports; source locations; the doc comment, raw and
      parsed.
- [ ] Doc comment formats, parsed into the document tree: Markdown,
      reStructuredText (with `lsdocs-rst`), Google and NumPy docstring
      sections, Doxygen / Javadoc / TSDoc tags, rustdoc's intra-doc links.
- [ ] Cross-references: qualified names to anchors, across languages and
      pages; the same object model as the Sphinx domains (11d); inventory
      export and import (intersphinx).
- [ ] Output: pages (one per module, or per item for large APIs, as the
      user chooses), and directives / Markdown blocks that place a single
      item in a hand-written page (`autoclass`-style).
- [ ] The plugin glue every language package uses: options (paths, which
      items, private items or not, page layout), generated documents,
      dependencies for the cache.
- [ ] Tests with a small fake language, so the framework is tested without
      any real extractor.

### 12c: Language packages
Each is a crate and plugin of its own (`plugins/apidoc-<language>`),
depending on the framework. Static extraction first: no project code runs.
- [ ] **Rust:** from rustdoc's JSON output (needs a nightly toolchain, and
      the format is unstable: pin and check its version), or from `syn`
      without type information. Decide early.
- [ ] **Python:** a Rust Python parser (e.g. ruff's) for modules and stub
      files, with type annotations and docstrings; later, optional runtime
      inspection for extension modules.
- [ ] **TypeScript:** a Rust TypeScript parser (e.g. oxc) for `.ts` and
      `.d.ts` files, with TSDoc; types as written, without the type checker.
- [ ] **C/C++:** libclang (accurate, needs LLVM installed) or tree-sitter
      (no dependency, less exact). Decide early; Doxygen comments either
      way.

**Exit:** the lsdocs docs site documents `lsdocs-plugin-sdk` (Rust), the
Python and Node.js bindings, and a C/C++ sample, with links between the
generated pages and the guides.

## Phase 13: HTML themes

**Goal:** themes configured by settings, not one renderer per theme
variant, with the [zensical theme](https://github.com/zensical/ui) as the
flagship. Today's default theme stays as the simple built-in one.

### 13a: The settings scheme
- [ ] A theme is a directory: templates (minijinja, with `extends` and
      blocks), partials, static assets, and a manifest declaring its
      settings (with a JSON Schema, for validation and defaults) and the
      data it needs.
- [ ] Site configuration: the theme to use (a built-in name or a
      directory), its settings, and overrides (a directory whose templates
      and assets replace or extend the theme's, block by block). Decide
      whether this lives in `render-html`'s options or in `[site]` / a
      `[theme]` table; `site.theme` is already reserved.
- [ ] Settings every theme should take: palette (light, dark or following
      the system; primary and accent colors), fonts, logo and favicon,
      language and translatable labels, extra CSS and JS, social links,
      footer text, edit-this-page links.
- [ ] Assets: copying the theme's assets, fingerprinting, minifying where
      cheap.

### 13b: Data for themes
- [ ] The table of contents as data for templates (a sidebar), not only
      inserted into the content: `transform-toc` writes it to metadata as
      well, or the renderer builds it.
- [ ] Search: an index written at `render/site` (pages, sections, text), and
      a client-side search the theme can use.
- [ ] Content components themes style: content tabs, buttons, icons and
      emoji, code block titles, copy buttons, highlighted lines. Each needs
      a node kind or a `custom` convention, and Markdown and
      reStructuredText syntax (tabs: a `tab-set`-style directive, and
      MkDocs-style `===` blocks).

### 13c: The zensical theme
- [ ] Research first. zensical/ui is MIT licensed and early-stage (its
      README says its API may change). Find its template engine and the
      data its templates expect from the generator, its settings (palette,
      feature flags, ...), and how its assets are built (Node 24, esbuild,
      Sass, PostCSS). Decide: port its templates to minijinja, or render
      them as they are if the engine is compatible; ship its built assets
      or build them.
- [ ] Port, keeping its license notice; map lsdocs nodes to the markup its
      styles expect (admonitions, code blocks, tables, tabs).
- [ ] Features, as the research finds them: navigation tabs and sections,
      a table-of-contents sidebar, search, a dark mode toggle, instant
      navigation, code copy and annotations, header and footer.
- [ ] Track upstream: pin a version, and check for upstream changes to the
      templates we ported.
- [ ] The docs site uses it.

**Exit:** the docs site builds with the zensical theme, configured only by
settings and a template override or two.

## Phase 14: Release

- [ ] Freeze protocol 1.0, after the release candidate has been used by the
      plugins of phases 10-13.
- [ ] Get CI running: it is written but has never run.
- [ ] Release CI (5c): wheels and per-platform npm packages for win/mac/
      linux on x64 and arm64, plus linux-musl; free-threaded Python wheels.
- [ ] Publish crates, wheels and npm packages under checked names.
- [ ] Host the docs site.

---

## TODO list

Things left open along the way, by area. The phases above may pick them up.

### Protocol and host
- [ ] Transformers receive whole documents; send only the subtrees a
      transformer selects, and accept patches back (decide with benchmarks).
- [ ] Site-wide data (the navigation) is sent with every page; share it
      once instead (an open decision, below).
- [ ] No restart policy: a plugin that fails to start, or dies, fails the
      build.
- [ ] Node.js binding: context events (logs, progress, diagnostics) reach
      the host when the handler finishes, not as they happen.

### Plugin SDK bindings
- [ ] `TypedDict`s (Python) and `.d.ts` types (Node.js) generated from
      `schema/` instead of written by hand, and type-checked against the
      fixtures.
- [ ] The binding packages declare their protocol version (now a string)
      in their package metadata.
- [ ] Free-threaded Python builds: never tried; consider a higher default
      `max_concurrency` there.

### `lsdocs-rst`
- [ ] `include :parser:` is reported as unsupported; docutils' standard
      include files (`<isonum.txt>`, `<s5defs.txt>`, ...) are not bundled
      (the path goes to the loader as written).
- [ ] `:encoding:` accepts every name Python knows, but only UTF-8
      (`utf-8-sig` too), Latin-1 and ASCII decode; others are an error.
- [ ] docutils prints `line="True"` for messages it reports with
      `line=None`; reproduced for the `class` directive's error only.
- [ ] `csv-table` cells: docutils numbers each cell's lines from 0, with
      the table's source; lsdocs-rst keeps each line's real origin. Only
      visible in messages about cell content.
- [ ] Absolute `include` paths: docutils makes them relative to the
      working directory; lsdocs-rst keeps them absolute. Expected trees for
      includes from subdirectories differ between Windows and other
      systems (path separators), so the conformance cases avoid them.
- [ ] A `.. end of inclusion from "x"` comment without an `include` makes
      docutils crash; lsdocs-rst ignores it.
- [ ] RCS keyword cleaning (docinfo) matches case-insensitively only for
      ASCII; Python's `re.IGNORECASE` also folds a few non-ASCII letters.
- [ ] `figure :figwidth: image`: docutils turns the URI into a path with
      `url2pathname` (percent-decoding); lsdocs-rst uses the URI as is.
- [ ] Messages for `:url:` failures are close to Python's, not identical.
- [ ] `nur fuzz rst` runs without file access, so includes are not fuzzed;
      add a mode with generated include files.
- [ ] PEP and RFC references, and most docutils settings, are not
      configurable yet (10b).
- [ ] Report docutils' `TargetNotes` infinite loop upstream: a paragraph
      with an embedded-URI reference and its own target, plus
      `target-notes`.

### `parser-rst`
- [ ] Inline semantics are lost: `subscript`, `superscript`,
      `abbreviation`, `acronym` and `inline` with classes become their
      text; `title_reference` becomes emphasis. Needs inline node kinds or
      an inline `custom` convention (renderers draw `custom` as a block
      `div`).
- [ ] Tables lose cell spans (`morecols`, `morerows`) and stub columns; the
      doctree's tables have neither.
- [ ] Images lose `width`, `height`, `scale` and `align`; figures lose
      `figwidth` and `align`.
- [ ] Topic and sidebar titles become bold paragraphs; nested line blocks
      are indented with no-break spaces.
- [ ] `:doc:` links to `<path>.rst` only, so it cannot point at a Markdown
      page.
- [ ] Option values of unknown directives are kept as strings.
- [ ] Diagnostics about included files have no span.
- [ ] Results are not cached while `file_access` is on (12a dependencies
      fix this).

### Rendering
- [ ] `render-html` draws `custom` nodes as an empty `div` with the kind as
      its class, so the content of unknown directives (`mermaid` and the
      like) is not shown unless a transformer handles it.

### Docs
- [ ] The docs site names `spec/wire.md` without linking to it, since pages
      cannot link outside the source directory. Publish the spec as a page
      (or generate one from it) and link it.

## Open decisions

| Decision | Leaning | Decide by |
|---|---|---|
| Transformers: whole tree vs. patches | whole tree; patches decided with benchmarks | before 1.0 |
| Site-wide data shared once (e.g. navigation) instead of per page | open | when nav cost matters |
| How Sphinx add-ons hook into parsing | a directive crate registered by `parser-rst`, plus a transformer | 11a |
| Rust API extraction: rustdoc JSON or `syn` | rustdoc JSON | 12c |
| C/C++ extraction: libclang or tree-sitter | open | 12c |
| Theme settings: `render-html` options or a `[theme]` table | open | 13a |
| zensical templates: port to minijinja or render as they are | research first | 13c |

Decided ones, with their reasons, are in [HISTORY.md](HISTORY.md) (phases 0-9)
or under their phase above.
