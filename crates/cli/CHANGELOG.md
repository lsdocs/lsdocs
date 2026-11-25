# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.1](https://github.com/lsdocs/lsdocs/compare/lsdocs/v0.1.0...lsdocs/v0.1.1) - 2026-11-25

### Added

- *(cli)* log build progress and add key commands to serve ([#22](https://github.com/lsdocs/lsdocs/pull/22))

## [0.1.0](https://github.com/lsdocs/lsdocs/releases/tag/lsdocs/v0.1.0) - 2026-11-23

### Added

- release automation ([#3](https://github.com/lsdocs/lsdocs/pull/3))
- tests/site, two CI jobs, ROADMAP after the split, README, AGENTS.md
- *(extras)* a plugin for lsdocs' own content features
- *(render-html)* search
- *(host)* mark generated documents
- *(render-html)* the default theme
- *(render-html)* themes, their settings and assets
- *(transform-toc)* the toc as page data
- *(render-html)* the lsd- markup contract
- *(host)* plugin settings in .config/ (Phase 13)
- *(apidoc-clang)* C and C++ API documentation with libclang (Phase 12c)
- *(apidoc-rust)* Rust API documentation from rustdoc's JSON (Phase 12c)
- *(apidoc-typescript)* TypeScript API documentation (Phase 12c)
- *(apidoc-python)* Python API documentation (Phase 12c)
- *(host)* result dependencies and site/parse (Phase 12a)
- *(sphinx-addons)* index page, toctree navigation and numbering, rubric (Phase 11)
- *(sphinx-addons)* syntax extensions and Sphinx's directives and roles (Phase 11)
- protocol 1.0-rc.0, finalized spec and docs site (Phase 9)
- *(parser-rst)* reStructuredText parser plugin (Phase 8e)
- lsdocs serve with incremental rebuilds and live reload (Phase 7c)
- content-hash build cache (Phase 7b)
- adopt the shared document shape in plugins (Phase 6d-3 to 6d-5)
- highlight code blocks with transform-highlight (Phase 6c)
- add site navigation and a sitemap (Phase 6b)
- check links, fill cross-references and add backlinks (Phase 6a)
- render a real site from CommonMark with templates (Phase 4)
- add plugin SDK, host pipeline and CLI (Phase 3)

### Fixed

- *(render-html)* theme fixes from review
- *(render-html)* group footnotes inside sections
- settings of the wrong kind are errors; stale options wording
- *(transform-links)* count label links as backlinks

### Other

- rename sdk-dynamic, built-ins behind features
- *(protocol)* rename pipeline methods (Phase 13)
- per-crate MSRV
- deny unwrap, expect and panic outside tests
- License lsdocs under Apache-2.0 OR MIT
- Scaffold the lsdocs workspace (roadmap Phase 0)
