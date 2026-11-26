# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.1](https://github.com/lsdocs/lsdocs/compare/lsdocs-host/v0.1.0...lsdocs-host/v0.1.1) - 2026-11-26

### Added

- *(host)* resolve links transformers add ([#27](https://github.com/lsdocs/lsdocs/pull/27))
- *(serve)* restart the plugins when their settings files change ([#25](https://github.com/lsdocs/lsdocs/pull/25))
- *(host)* prune what earlier builds wrote, across sessions ([#24](https://github.com/lsdocs/lsdocs/pull/24))
- *(cli)* log build progress and add key commands to serve ([#22](https://github.com/lsdocs/lsdocs/pull/22))

## [0.1.0](https://github.com/lsdocs/lsdocs/releases/tag/lsdocs-host/v0.1.0) - 2026-11-23

### Added

- release automation ([#3](https://github.com/lsdocs/lsdocs/pull/3))
- *(render-html)* search
- *(host)* mark generated documents
- *(host)* plugin settings in .config/ (Phase 13)
- *(apidoc)* the API documentation framework (Phase 12b)
- *(host)* result dependencies and site/parse (Phase 12a)
- *(sphinx-addons)* index page, toctree navigation and numbering, rubric (Phase 11)
- *(sphinx-addons)* syntax extensions and Sphinx's directives and roles (Phase 11)
- protocol 1.0-rc.0, finalized spec and docs site (Phase 9)
- *(parser-rst)* reStructuredText parser plugin (Phase 8e)
- lsdocs serve with incremental rebuilds and live reload (Phase 7c)
- content-hash build cache (Phase 7b)
- *(host)* normalize parsed documents (Phase 6d-2)
- add site navigation and a sitemap (Phase 6b)
- check links, fill cross-references and add backlinks (Phase 6a)
- render a real site from CommonMark with templates (Phase 4)
- add plugin SDK, host pipeline and CLI (Phase 3)

### Other

- the renamed pipeline methods
- *(protocol)* rename pipeline methods (Phase 13)
- deny unwrap, expect and panic outside tests
- parallel pipeline, faster node decoding, benchmarks (Phase 7a)
- License lsdocs under Apache-2.0 OR MIT
- Scaffold the lsdocs workspace (roadmap Phase 0)
