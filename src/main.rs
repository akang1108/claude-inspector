mod scan;
mod ui;

use std::path::PathBuf;

use anyhow::Result;

const HELP: &str = "\
claude-inspector: browse Claude Code memory across installs

USAGE:
  claude-inspector [OPTIONS]

OPTIONS:
  -c, --config-dir <DIR>  Add a Claude config dir (repeatable); listed first
      --no-discover       Skip auto-discovery of ~/.claude* siblings
      --list              Print a plain-text summary instead of the TUI
  -h, --help              Show this help

Honors CLAUDE_CONFIG_DIR, falling back to ~/.claude.
";

fn main() -> Result<()> {
    let mut explicit = Vec::new();
    let mut discover = true;
    let mut list = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-c" | "--config-dir" => match args.next() {
                Some(v) => explicit.push(PathBuf::from(v)),
                None => anyhow::bail!("{a} needs a value"),
            },
            "--no-discover" => discover = false,
            "--list" => list = true,
            "-h" | "--help" => {
                print!("{HELP}");
                return Ok(());
            }
            other => anyhow::bail!("unknown argument: {other}\n\n{HELP}"),
        }
    }

    let installs: Vec<_> = scan::resolve_dirs(explicit, discover)
        .into_iter()
        .map(|(d, env)| scan::scan(&d, env))
        .collect();

    if list {
        print_list(&installs);
        return Ok(());
    }
    ui::run(installs)
}

fn print_list(installs: &[scan::Install]) {
    for i in installs {
        println!("== {}{}", scan::tilde(&i.dir), if i.from_env { "  (CLAUDE_CONFIG_DIR)" } else { "" });
        for s in scan::Scope::ALL {
            let items: Vec<_> = i.entries.iter().filter(|e| e.scope == s).collect();
            if items.is_empty() {
                continue;
            }
            println!("  {} ({})", s.title(), items.len());
            let mut last = "";
            for e in items {
                if s.per_project() && e.group != last {
                    println!("    {}", e.group);
                    last = &e.group;
                }
                println!("      {}  [{}]", e.name, ui::human_size(e.size));
            }
        }
    }
}
