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

Phases 0-11 are done (0-9 are recorded in [HISTORY.md](HISTORY.md)):

| Phase | Result |
|---|---|
| 0-3 | Workspace, data crates (`doctree`, `config`, `protocol`), msgpack-rpc layer, plugin SDK and host. |
| 4 | A real site: `parser-markdown`, `transform-toc`, `render-html`. |
| 5 | Python and Node.js plugin bindings (packaging deferred, 5c). |
| 6 | Links, navigation, sitemap, syntax highlighting; the shared document shape. |
| 7 | Parallel pipeline, build cache, `lsdocs serve`, binding performance. |
| 8 | `lsdocs-rst`, a docutils port checked against docutils (parser, directives, transforms, file access), and `parser-rst`. |
| 9 | Protocol `1.0-rc.0` and its compatibility policy, the finished spec, a plugin written from the spec alone, and the docs site. |
| 10 | `lsdocs-rst` from Python and Node.js: docutils' settings, a serde form of the tree and the XML writer, and the `lsdocs-rst` / `@lsdocs/rst` packages (below). |
| 11 | Syntax extensions in the protocol and both parsers, generated documents, and the `sphinx-addons` plugin: Sphinx's directives and roles in reStructuredText and Markdown, toctree navigation and numbering, the index page. |

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
    sphinx-addons/  Sphinx's directives and roles          (planned, Phase 11)
    apidoc-python/  apidoc-typescript/  apidoc-cpp/  apidoc-rust/
                                                            (planned, Phase 12)
  bindings/
    plugin/python/  plugin/node/    the plugin SDK bindings
    rst/python/     rst/node/       lsdocs-rst bindings
  docs/  examples/  schema/  fixtures/  spec/  scripts/  tools/
```

Package names are working names; check them on crates.io, PyPI and npm before
publishing anything.

---

## Phase 10: The reStructuredText library, from other languages (done)

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
- [x] Settings: one `Settings` type for the docutils settings the parser
      honors (`doctitle`, `report_level`, `pep_references`,
      `rfc_references`, `tab_width`, `line_length_limit`, `smart_quotes`,
      `sectsubtitle_xform`, `toc_backlinks`, `auto_id_prefix`, ...), with
      docutils' defaults; the builder methods stay as shorthands.
- [x] Stable tree accessors (kinds, attributes, children, spans) and a
      serde representation of the tree, so bindings and tools can exchange
      it as JSON or MessagePack.
- [x] Writers: pseudo-XML (exists), and docutils' XML writer, checked
      against docutils. HTML is lsdocs' job, not the library's.
- [x] rustdoc for every public item, with examples; the crate's README.

Decisions:
- `Settings` covers the settings that change the tree `publish_doctree`
  builds: `doctitle_xform`, `docinfo_xform`, `sectsubtitle_xform`,
  `sectnum_xform`, `title`, `tab_width`, `line_length_limit`,
  `raw_enabled`, `pep_references` (with `pep_base_url`,
  `pep_file_url_template`), `rfc_references` (with `rfc_base_url`),
  `trim_footnote_reference_space`, `toc_backlinks`, `id_prefix`,
  `auto_id_prefix` and `strip_comments`. Fields have docutils' names and
  defaults, so with `serde` a docutils configuration maps onto them
  directly (unknown names are an error). `report_level`, `halt_level` and
  the writer-side settings (`strip_classes`, `strip_elements_with_classes`)
  don't change that tree, so they are left out; `file_insertion_enabled`
  stays the presence of a `FileLoader`. The rest is on the TODO list.
- A conformance case may have a `<name>.settings.json`, read by docutils
  and lsdocs-rst alike; ten `settings_*` cases cover the settings.
- serde is an optional feature (`serde`), so the library keeps one
  dependency by default. The JSON form: elements are
  `{kind, attributes?, children?, span?}`, text is `{text, span?}`,
  attribute values are plain JSON values.
- The XML writer is checked on every case (`<name>.xml`, written by
  docutils' `XMLTranslator` with `indents` on, from the transformed tree).
  Its generator comment names lsdocs-rst instead of docutils; raw XML is
  not checked for well-formedness.

### 10c: Python package (`bindings/rst/python`)
- [x] pyo3 + maturin, abi3. `parse(text, **settings)` returns a tree of
      Python objects (kind, attributes, children, span), with `pformat()`
      and `to_dict()`.
- [x] Directives and roles written in Python, registered on a parser (the
      `Directive` and `Role` traits over Python callables), and a file
      loader callback.
- [x] Optional: conversion to a real `docutils.nodes.document`, for use
      with docutils writers and Sphinx, if docutils is installed.
- [x] Tests: the conformance cases run through the package too.

Decisions:
- The Python tree is plain Python objects (`Element`, `Text`), built from
  the crate's serde form, so it is mutable and needs no Rust to read.
  Writers (`pformat`, `to_xml`) take it back the same way.
- A Python directive's context methods (`parse_content`, `parse_inline`,
  `message`) need the parser's `DirectiveContext`, which borrows the
  parse. Instead of `unsafe` lifetime erasure, the Python callable runs on
  a scoped worker thread and the parsing thread answers its requests over
  a channel until it returns; the context releases the GIL while waiting,
  so Python directives nest. A context used after its directive returned
  raises `RuntimeError`.
- `DirectiveError` (and, for loaders, `OSError`) are reported in the
  document as docutils would; any other exception from a directive, role
  or loader is raised from `parse()` once the parse is done.
- Option kinds are given by docutils' conversion function names (or the
  functions themselves), so docutils directive classes port with few
  changes.
- `to_docutils()` converts kinds by docutils node class name; `pending`
  elements lose their transform, so convert transformed trees.

### 10d: Node.js package (`bindings/rst/node`)
- [x] napi-rs: `parse(text, settings)` returning plain objects, with types
      (`.d.ts`) for the tree and settings; directives, roles and the file
      loader as JS functions.
- [x] Tests: the conformance cases run through the package too.

Decisions:
- The tree is plain JSON-shaped data (`{kind, attributes?, children?,
  span?}`, `{text, span?}`), typed in `index.d.ts`, with helper functions
  (`findAll`, `traverse`, `astext`, `pformat`, `toXml`) instead of
  classes. Values cross the addon as JSON strings, as in the plugin
  binding.
- Parsing is synchronous. JS runs on one thread, so the GIL trick of the
  Python package doesn't apply: `parse()` runs the parser on a scoped
  worker thread and the JS thread serves it, running JS directives, roles
  and loaders when the worker asks; a directive's context sends its parse
  requests to the worker and keeps serving JS calls (nested directives)
  until the answer comes. The threads take turns like one call stack.
- Exceptions stay on the JS side: the wrapper turns a `DirectiveError`
  into a message, a Node.js system error (`code` `E...`) from a loader into
  a failed read, and keeps any other exception to throw from `parse()`.
- Directive specs use the same option kind names as the Python package
  (docutils' conversion function names).

**Exit:** both packages parse the conformance cases to docutils' trees, and
a directive written in each language works. Met: the pytest and
`node --test` suites run every case (parser, transforms and XML), and each
has directives that parse their content, nest, report errors and take
options.

## Phase 11: Sphinx add-ons

**Goal:** a `sphinx-addons` plugin implementing the directives and roles
Sphinx adds on top of docutils, so Sphinx projects build with lsdocs with
few changes. It is an lsdocs plugin working on the shared document shape,
not a port of Sphinx's extensions to `lsdocs-rst`: the same directives and
roles work in reStructuredText and in Markdown (MyST-style syntax).
Domains (object descriptions such as `py:function` and their roles) belong
to the API documentation framework (Phase 12).

### 11a: Syntax extensions
- [x] Decide how the add-ons hook in. Directives and roles are syntax,
      seen while parsing, but their meaning is the plugin's.
- [x] Protocol: a plugin declares the directives and roles it handles; the
      host passes them to parsers, which turn that syntax into neutral
      nodes for the plugin to transform.
- [x] `parser-rst` registers declared directives and roles with
      `lsdocs-rst`; `parser-markdown` reads MyST-style directives
      (```` ```{name} ```` fences with `:option:` lines) and roles
      (`` {name}`text` ``).
- [x] The plugin SDK declares syntax (`Plugin::syntax`).
- [x] The `sphinx-addons` plugin, with its options in `lsdocs.toml`.
      `:ref:` and `:doc:` move there from `parser-rst`.

Decisions:
- **Declared syntax.** A plugin lists the directives and roles it handles
  in a new capability, `syntax` (each directive with its arguments,
  options and whether its content is markup, text or nothing; each role
  by name). The host merges every plugin's list (the first plugin in
  config order wins a name) and sends it in every `parse` request
  (`ParseParams.syntax`), so it is part of the parse cache key. Not in
  `initialize`: plugins start in config order, so a parser could start
  before the plugins it should hear about; and not in a message of its
  own, so parsers stay stateless. Both fields are optional, so the
  change is additive within `1.0-rc.0`.
- **Neutral nodes.** A declared directive becomes a `custom` node of its
  name: `argument`, `options` (flags `true`, values as text) and text
  `content` as attributes, markup content parsed into its children, and
  a markup argument parsed into a first `directive_argument` child. A
  declared role becomes a `custom` node of kind `role` with `name` and
  `text`, and the text as a child so plain-text views (titles, ids) are
  right. Written down in "Syntax extensions" in `spec/wire.md`.
- **Markdown syntax is MyST's**: fences (backticks or tildes) whose
  language is `{name}`, options as `:name: value` lines or a `---` YAML
  block, roles as `{name}` right before inline code. Directive content is
  parsed as Markdown on its own, with its spans moved to the file. An
  undeclared `{name}` fence becomes a `custom` node as an unknown
  reStructuredText directive does; an undeclared role stays text and
  code.
- **One transformer.** `sphinx-addons` expands directives in both passes
  (expansion depends only on the page, the options and included files),
  collects labels, terms, figure and equation numbers, todos, section
  trees and toctree entries, and resolves roles, toctrees and todo lists
  in the apply pass. It lists before the other transformers, which then
  see links and code blocks. (Its results were not cached while
  `file_access` was on; 12a's dependencies fixed that.)
- **Checked against Sphinx's behavior, not its doctrees.** The output is
  lsdocs' document shape, so the tests check what Sphinx does (titles,
  numbering, line selection, the same tree from reStructuredText and from
  Markdown), not doctree equality.
- Labels: a `ref` target is used as written if a page has that anchor,
  else as docutils makes ids (`My Label` is `my-label`). `doc` finds a
  page in any format (`.rst`, `.rest`, `.md`, `.markdown`).
- Inline semantics without new node kinds: inline roles become core
  inlines where one fits (`command` strong, `dfn` emphasis, `regexp` code)
  and otherwise `custom` nodes with `inline: true` (and `html_tag`,
  `title`), which `render-html` draws as inline elements.

### 11b: Directives
- [x] Code: `code-block` / `sourcecode` (`:linenos:`, `:emphasize-lines:`,
      `:caption:`, `:name:`, `:dedent:`, `:force:`), `literalinclude`
      (`:lines:`, `:start-after:`, `:end-before:`, ...), `highlight`.
      `:pyobject:` and `:diff:` are reported as unsupported.
- [x] Notes: `versionadded`, `versionchanged`, `deprecated`,
      `versionremoved`, `seealso`, `todo` / `todolist` (sphinx.ext.todo).
- [x] Content: `only` (by tags), `centered`, `hlist`, `glossary`,
      `tabularcolumns` (accepted, ignored), `sectionauthor` /
      `codeauthor`, `acks`.
- [x] `rubric` with Sphinx's options (`:heading-level:`).
- [x] `toctree` (`:maxdepth:`, `:caption:`, `:hidden:`, `:glob:`,
      `:titlesonly:`, `:reversed:`) as a list of links to the pages it
      names and their sections.
- [x] `toctree` feeding `transform-nav`: a site whose pages have toctrees
      gets its navigation from them instead of from its directories; and
      `:numbered:` (section numbers, `numref` to sections).
- [x] `index` directive and role, and a generated index page (with the
      generated documents of 12a, done here).
- [x] Math: `math` with `:label:` and equation numbering, `:eq:`.

Decisions:
- Version notes are admonitions titled as Sphinx 7.3+ words them ("Added
  in version 2.0"), with the explanation as their first paragraph.
- `literalinclude` takes the language from the file's extension when
  `:language:` is not given (Sphinx uses `highlight_language`);
  `transform-highlight` knows languages by extension.
- `:linenos:`, `:lineno-start:` and `:emphasize-lines:` go into the code
  block's `info` (`linenos lineno-start=3 emphasize-lines=1,2`); nothing
  draws them yet (13b).
- `numfig` numbers figures, tables and code blocks with a caption per
  page (no section numbers), and prefixes their captions ("Fig. 1 ").
  Numbered equations get `\tag{n}`, which KaTeX and MathJax draw.
- **Generated documents** (from 12a, needed for the index page; breaking
  the release-candidate protocol is fine): `transform/collected` now
  returns documents the plugin generates from what it collected
  (`Transformer::generate` in the Rust SDK). The host checks their paths
  are free, normalizes them, and they go through the plugin's apply
  pass, later transformers and rendering. A plugin that does not collect
  cannot generate; 12a can add a step of its own if a generator needs
  none.
- **Toctrees drive the navigation through a convention, not a coupling:**
  `transform-nav` follows `nav_children` (a page's ordered child pages)
  when the root `index` page has it, and `sphinx-addons` sets it from
  toctrees (hidden ones too). Front matter can set it as well.
- `:numbered:` numbers the pages a toctree names (1, 2, ...), their
  sections below that, and the pages their own toctrees name after
  their sections (Sphinx places those where the toctree is). The number
  goes into the heading's text (`1.2. Title`), so references that take
  the title show it.
- The index page is `genindex.md` (`genindex.html`), generated only if a
  page has index entries, and out of the navigation (`nav_exclude`);
  entries are grouped by first letter, symbols last. `rubric` overrides
  docutils' (a `custom:rubric` with `html_tag` `p` or `h1`-`h6`;
  `render-html` honors `html_tag` on block custom nodes too).

### 11c: Roles
- [x] Cross-references: `ref`, `doc`, `numref` (with `numfig` numbering),
      `term`, `eq`, `any`, `download` (a link; the file is copied as any
      asset is).
- [x] Inline semantics: `abbr`, `command`, `dfn`, `file`, `guilabel`, `kbd`,
      `mailheader`, `makevar`, `manpage`, `menuselection`, `mimetype`,
      `newsgroup`, `program`, `regexp`, `samp`.
- [x] `extlinks`-style link shortcuts (roles declared from the plugin's
      options).

**Exit:** every directive and role above works from reStructuredText and
from Markdown in a test site, and a mid-sized Sphinx project (to be
picked) builds without errors. Met: the plugin's tests build the same
pages from both formats, and the sample site, which uses both, stands in
for a Sphinx project until the HTML themes (Phase 13) make a real one
worth comparing.

## Phase 12: API documentation framework

**Goal:** documentation generated from source code for Python, TypeScript,
C/C++ and Rust. A language-neutral framework does the common work; each
language is its own package that depends on it and only extracts.

### 12a: Platform additions
- [x] **Generated documents:** done in Phase 11 (`transform/collected`
      returns them; see 11b's decisions). Revisit if a generator needs no
      collect pass.
- [ ] Generated documents from the Python and Node.js plugin SDKs (only
      the Rust SDK has `Transformer::generate`).
- [x] **Dependencies:** parsers and generators report the files a result
      depends on (source files read, included files), so the cache and
      `lsdocs serve` invalidate exactly; this replaces turning caching off.
- [x] **Parsing through the host:** a plugin asks the host to parse a piece
      of text with the parser of a file extension (`site/parse`), so doc
      comments in Markdown or reStructuredText become document nodes
      without the framework depending on any parser.

Decisions:
- `dependencies` (on parse, collect, apply and collected results) are
  absolute paths or paths relative to the site directory. A cache entry
  records a digest of each (or that it is missing) and is reused only
  while they match; the cache format is now `lsdocs-cache-2`. A build
  reports them (`BuildReport::dependencies`) and `lsdocs serve` watches
  them besides the site directory. `parser-rst` (`include`, `:file:`)
  and `sphinx-addons` (`literalinclude`) report the files they read and
  are cached again with `file_access` on.
- `site/parse` is the first plugin -> host request. The host answers it
  with the first parser taking the extension, with the site's syntax
  extensions, without normalizing. In the Rust SDK it is
  `Context::parse`; in-process plugins reach the host through their
  event sink (`EventSink::parse`), remote ones through the connection.
  A parser asking for its own extension at its `max_concurrency` would
  deadlock; noted in the spec, not guarded.

### 12b: The framework (`crates/apidoc`)
- [x] The model: modules, namespaces and packages; classes, structs,
      interfaces, traits, enums and variants; functions, methods and
      overloads; fields and properties; constants, type aliases, macros.
      Signatures with parameters, defaults, types and generics; visibility,
      inheritance, re-exports; source locations; the doc comment, raw.
- [x] Doc comment formats: Google and NumPy docstring sections, Sphinx
      field lists, Doxygen / Javadoc / TSDoc tags, split out by the
      framework; their text (Markdown, reStructuredText) parsed into the
      document tree by the site's parsers (`site/parse`).
- [ ] rustdoc's intra-doc links (with the Rust package).
- [x] Cross-references: qualified names to anchors, across pages, in the
      scope a reference is written in.
- [ ] Inventory export and import (the `ext-ref` TODO).
- [x] Domains: object description directives (`.. py:function::`, with
      their signatures read), `module` / `currentmodule`, and roles
      (`:py:func:`, `:cpp:class:`), declared as syntax extensions so they
      work in reStructuredText and Markdown; hand-written and generated
      items share one index.
- [ ] The domains with no extractor: `rst` (`rst:directive`, `rst:role`)
      and `std`'s `option` / `program`, `envvar` and `productionlist` /
      `token`. (`py`, `c`, `cpp`, `js` come with their language packages.)
- [x] Output: a page per module, plus an index page; `<domain>:auto`
      places an item (with its members) in a hand-written page.
- [ ] A page per item, for large APIs (the user's choice).
- [x] The plugin glue every language package uses: options (output,
      private items, doc markup and style), generated documents,
      dependencies for the cache.
- [x] Tests with a small fake language, so the framework is tested without
      any real extractor.

Decisions:
- **Two crates.** `lsdocs-apidoc` is standalone (serde only): the model,
  docstring sections, the index and name resolution, anchors. Rendering
  is lsdocs' job, in `lsdocs-apidoc-plugin`, which language packages
  build on: they implement `Language` (domain, extraction, signature
  format and reading, the domain's object directives and roles) and get
  a plugin from `apidoc_plugin::plugin`.
- **Doc comment markup goes through the site's parsers** (`site/parse`,
  12a): the framework splits the docstring sections, and each piece is
  parsed by the parser of the language's doc format (`md` or `rst`),
  with the site's syntax extensions, so roles in docstrings work.
- **Anchors are `<domain>.<qualified name>`** (`py.pkg.mod.Class`), case
  kept. References resolve in their scope (the item, then each scope
  around it), then as the end of exactly one qualified name; an
  ambiguous one is a warning and takes the first; a missing one is
  shown as code without a warning (as Sphinx without `nitpicky`).
- **Generated pages** are `<output>/<qualified name>.md` (default output
  `api/<domain>`), one per module, each item a section (its heading the
  kind and name, then the signature as a code block in the language,
  then its documentation, then its members); `<output>/index.md` lists
  the modules. Hand-placed items (`auto`, object directives) are
  `custom:api-item` blocks without headings, so they don't change the
  page's sections.
- The pages come from `transform/collected` (11b), so extraction runs
  once per build; its files are the result's dependencies. Apply results
  depend on the generated pages too (the host's apply cache key now
  covers generated documents), since references resolve against them.
- Signatures in a generic notation (`name(a: int = 1) -> str`) by
  default; languages override both writing and reading them.

### 12c: Language packages
Each is a crate and plugin of its own (`plugins/apidoc-<language>`),
depending on the framework. Static extraction first: no project code runs.
- [ ] **Rust:** from rustdoc's JSON output (needs a nightly toolchain, and
      the format is unstable: pin and check its version), or from `syn`
      without type information. Decide early.
- [x] **Python:** a Rust Python parser (ruff's) for modules and stub
      files, with type annotations and docstrings (`apidoc-python`).
- [ ] Python: re-exports (`from ._native import Context` in a package's
      `__init__` documents nothing; Sphinx's autodoc imports and would
      show it), and optional runtime inspection for extension modules.

Decisions (Python):
- **ruff's parser, and MSRV 1.97.** `ruff_python_parser` /
  `ruff_python_ast` / `ruff_text_size` are on crates.io as Astral's
  internal crates: current with Python's syntax, but every release needs
  Rust 1.95+, so the workspace MSRV went from 1.89 to 1.97 (the user's
  call), and they are pinned exactly (`=0.0.22`) since their API may
  change in any release.
- Statically, like the rest: packages from directories (`__init__`,
  modules, subpackages; `_`-names skipped), a `.pyi` stub instead of the
  module beside it, the first `@overload`. A class's signature is its
  `__init__`'s; `self.x` assignments in `__init__` with an annotation or
  a docstring are fields; attribute docstrings (a string after an
  assignment) document variables. `__all__`, `_`-names and undocumented
  special methods decide what is private.
- Docstrings are reStructuredText by default, as in most Python code.
  The framework option `primary` declares the roles without their domain
  prefix (`:func:`), as Sphinx's `primary_domain` makes them work; only
  roles, since unprefixed directives (`class`) clash with docutils'.
- The docs site documents the Python plugin SDK and `lsdocs-rst` (under
  `api/`), part of this phase's exit.
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

### `lsdocs-rst` bindings
Their known limitations are listed in each package's README
(`bindings/rst/python/README.md`, `bindings/rst/node/README.md`); keep
those lists current. To fix:
- [ ] Python: exceptions from extensions are collected per `Parser`, so
      two threads parsing with one `Parser` at once can get each other's
      exception. Collect them per `parse()` call.
- [ ] Both: extensions can't be async (Node.js) or run on the calling
      thread's context (Python directives run on a worker thread).

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
- [ ] Settings not supported yet: `smart_quotes` (and
      `smartquotes_locales`), `character_level_inline_markup`,
      `language_code` (labels and bibliographic field names are English),
      `root_prefix` (absolute include paths), `syntax_highlight` (`code`
      blocks are always as with `none`), `expose_internals`, and the
      `Decorations` settings (`generator`, `datestamp`, `source_link`,
      `source_url`).
- [ ] PEP and RFC references in plain text match ASCII digits only;
      Python's `\d` also matches other Unicode decimal digits.
- [ ] The XML writer doesn't check raw XML for well-formedness (docutils
      warns about invalid raw XML).
- [ ] Report docutils' `TargetNotes` infinite loop upstream: a paragraph
      with an embedded-URI reference and its own target, plus
      `target-notes`.

### `parser-rst`
- [ ] Take docutils settings from the plugin's options (a `settings`
      table deserialized into `lsdocs_rst::Settings`).
- [ ] Inline semantics are lost: `subscript`, `superscript`,
      `abbreviation`, `acronym` and `inline` with classes become their
      text; `title_reference` becomes emphasis. The inline `custom`
      convention `sphinx-addons` uses (`inline: true`, `html_tag`) would
      do.
- [ ] Tables lose cell spans (`morecols`, `morerows`) and stub columns; the
      doctree's tables have neither.
- [ ] Images lose `width`, `height`, `scale` and `align`; figures lose
      `figwidth` and `align`.
- [ ] Topic and sidebar titles become bold paragraphs; nested line blocks
      are indented with no-break spaces.
- [ ] Option values of unknown directives are kept as strings.
- [ ] Diagnostics about included files have no span.

### `sphinx-addons` and syntax extensions
- [ ] The Python and Node.js plugin SDKs cannot declare syntax yet
      (`Plugin::syntax` is Rust only).
- [ ] Markdown: MyST's colon fences (`:::{name}`) are not read
      (pulldown-cmark has no hook for them), nor MyST's `(label)=`
      targets. Directive content is parsed on its own, so it cannot use
      the page's link reference definitions or footnotes.
- [ ] Documents are found by extension (`.rst`, `.rest`, `.md`,
      `.markdown`) for `doc` and `toctree`, since a transformer does not
      know which files parsers took.
- [ ] Not supported: `literalinclude`'s `:pyobject:` and `:diff:`, encodings
      other than UTF-8; compound keys in `kbd` (`Ctrl+C` is one key);
      several terms for one glossary entry (in reStructuredText the
      content is a definition list); `conf.py` settings that change
      parsing (`default_role`, `rst_prolog` / `rst_epilog`, the
      `|version|` and `|release|` substitutions).

### Plugins to write
- [ ] `ext-ref`: references to other sites' objects and labels, read from
      inventories (Sphinx's `objects.inv` as one format among others, not
      the only one), and an inventory written for the lsdocs site, so
      other sites can link to it. Replaces intersphinx; not specific to
      Sphinx.

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
| How Sphinx add-ons hook into parsing | declared syntax: parsers make neutral nodes, a transformer gives them meaning | 11a |
| Rust API extraction: rustdoc JSON or `syn` | rustdoc JSON | 12c |
| C/C++ extraction: libclang or tree-sitter | open | 12c |
| Theme settings: `render-html` options or a `[theme]` table | open | 13a |
| zensical templates: port to minijinja or render as they are | research first | 13c |

Decided ones, with their reasons, are in [HISTORY.md](HISTORY.md) (phases 0-9)
or under their phase above.
