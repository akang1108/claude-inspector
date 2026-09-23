# claude-inspector

TUI to browse Claude Code memory across config dirs.

## Usage

- Run `make` to list build, run, and test targets
- Run `make run` to open the TUI

## Config dirs

- Honors `CLAUDE_CONFIG_DIR`, else `~/.claude`
- Add more with `-c <dir>` (repeatable), e.g. `make run ARGS="-c ~/.claude-me"`
- Sibling `~/.claude*` installs are auto-discovered; disable with `--no-discover`

## Keys

| Key               | Action           |
| :---------------- | :--------------- |
| `j` / `k`         | Move             |
| `J` / `K`         | Scroll preview   |
| `Tab` / `h` / `l` | Switch scope tab |
| `[` / `]`         | Switch install   |
| `/`               | Text filter      |
| `r`               | Rescan           |
| `q`               | Quit             |
