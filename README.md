# lsdocs

A static-site generator whose parsers, transformers and renderers are
**plugins** talking to a **host** over an LSP-style protocol: msgpack-rpc
messages, length-prefixed, over stdio or a named pipe / unix socket.
First-party plugins also run in-process, with no serialization. Plugins
can be written in Rust, Python or Node.js.

This repository holds the host and the `lsdocs` command line tool
(`build`, `serve`, `init`); the protocol, the SDKs, the parsers and the
other plugins are the organization's other repositories
(https://github.com/lsdocs; the map is in ROADMAP.md, "The split").
Documentation: https://lsdocs.github.io.

Progress is logged to stderr, one line per pipeline phase; `-q` shows
only warnings and errors, `-v` adds debug output and timings, `-vv`
traces each document, and `LSDOCS_LOG` (an `EnvFilter` directive) replaces
all three. In a terminal, `lsdocs serve` takes single-key commands: `r`
restarts the plugins and rebuilds, `d` switches debug output on and off,
`h` lists the keys, and `q` quits.

The built-in plugins are default features of the `lsdocs` crate, one per
builtin (`parser-myst`, `render-html`, `apidoc-python`, ...); without
them (`--no-default-features`) lsdocs builds with none.

## Known limitations

- No restart policy: a plugin that fails to start, or dies, fails the
  build.
- `lsdocs serve` reloads a theme directory or its overrides only after a
  restart: the `r` key, a change under `.config/`, or a new run.
- `lsdocs serve` reads keys only when stdin and stderr are both a
  terminal; otherwise Ctrl+C stops it, as before. Progress is logged one
  line per phase (`start`, `parse`, `transform`, `render`, `write`), not
  per document (`-vv` logs each document).

## TODO

The open items of the host and the CLI, and the organization's, are in
ROADMAP.md's TODO list.
