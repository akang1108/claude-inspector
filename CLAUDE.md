# claude-inspector

- Rust TUI that inspects Claude Code settings and context, including hidden memory
- Usage and config dirs: see [README.md](README.md)
- Run `make` for build, test, and lint targets

## Layout

- `src/main.rs`: arg parsing and `--list` output
- `src/scan.rs`: filesystem scan into `Install` and `Entry` values; no terminal code
- `src/ui.rs`: ratatui app; one tab per `Scope`

## Rules

- Strictly read-only: never write outside this repo
- Never read or print env var values
  - `CLAUDE_CONFIG_DIR` is only read by name at runtime
- Add a new memory location as a `Scope` variant in `scan.rs`
  - Update `Scope::ALL`, `title`, and `scope_color` in `ui.rs`
- Keep dependencies minimal: no clap, no chrono

## Gotchas

- Project dir names under `projects/` are lossy; real cwd comes from transcript `cwd`
- Session transcripts are read only for cwd, titles, and preview
- Plugin-provided skills and agents are not scanned
