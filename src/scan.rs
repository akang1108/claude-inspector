use std::collections::HashSet;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

const MAX_READ: u64 = 4 * 1024 * 1024;
const MAX_RENDER: usize = 200 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scope {
    Managed,
    Global,
    Repo,
    ProjectMemory,
    Session,
}

impl Scope {
    pub const ALL: [Scope; 5] = [
        Scope::Managed,
        Scope::Global,
        Scope::Repo,
        Scope::ProjectMemory,
        Scope::Session,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Scope::Managed => "Managed policy",
            Scope::Global => "Global (user)",
            Scope::Repo => "Repo instructions",
            Scope::ProjectMemory => "Auto memory",
            Scope::Session => "Sessions",
        }
    }

    pub fn per_project(self) -> bool {
        matches!(self, Scope::Repo | Scope::ProjectMemory | Scope::Session)
    }
}

pub struct Entry {
    pub scope: Scope,
    pub group: String,
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

pub struct Install {
    pub dir: PathBuf,
    pub from_env: bool,
    pub entries: Vec<Entry>,
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

pub fn tilde(p: &Path) -> String {
    let h = home();
    match p.strip_prefix(&h) {
        Ok(rest) if !h.as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => p.display().to_string(),
    }
}

/// Explicit dirs win, then CLAUDE_CONFIG_DIR, then ~/.claude; siblings are auto-discovered unless disabled.
pub fn resolve_dirs(explicit: Vec<PathBuf>, discover: bool) -> Vec<(PathBuf, bool)> {
    let mut out: Vec<(PathBuf, bool)> = Vec::new();
    let mut seen = HashSet::new();
    let mut push = |p: PathBuf, env: bool, out: &mut Vec<(PathBuf, bool)>| {
        let key = fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
        if seen.insert(key) {
            out.push((p, env));
        }
    };

    for p in explicit {
        push(p, false, &mut out);
    }
    let env_dir = std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let has_env = env_dir.is_some();
    if let Some(p) = env_dir {
        push(p, true, &mut out);
    }
    let default = home().join(".claude");
    if !has_env {
        push(default.clone(), false, &mut out);
    }
    if discover {
        let mut found: Vec<PathBuf> = fs::read_dir(home())
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(".claude") && n != ".claude.json")
                    && p.join("projects").is_dir()
                    && (p.join("settings.json").is_file() || p.join("history.jsonl").is_file())
            })
            .collect();
        found.sort();
        for p in found {
            push(p, false, &mut out);
        }
    }
    if out.is_empty() {
        out.push((default, false));
    }
    out
}

pub fn scan(dir: &Path, from_env: bool) -> Install {
    let mut entries = Vec::new();

    for p in [
        "/Library/Application Support/ClaudeCode/CLAUDE.md",
        "/etc/claude-code/CLAUDE.md",
    ] {
        add_file(&mut entries, Scope::Managed, "", p.into(), p.to_string());
    }

    add_file(&mut entries, Scope::Global, "", dir.join("CLAUDE.md"), "CLAUDE.md".into());
    for f in md_files_recursive(&dir.join("rules")) {
        let name = format!("rules/{}", f.strip_prefix(dir.join("rules")).unwrap_or(&f).display());
        add_file(&mut entries, Scope::Global, "", f, name);
    }

    let mut repos_seen = HashSet::new();
    let mut projects: Vec<PathBuf> = fs::read_dir(dir.join("projects"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    projects.sort();

    for pdir in projects {
        let slug = pdir.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let sessions = session_files(&pdir);
        let cwd = sessions.iter().take(5).find_map(|(p, _)| project_cwd(p));
        let group = cwd.clone().unwrap_or_else(|| slug.clone());

        let mut mem = md_files_recursive(&pdir.join("memory"));
        mem.sort_by_key(|p| (p.file_name().is_none_or(|n| n != "MEMORY.md"), p.clone()));
        for f in mem {
            let name = f.strip_prefix(pdir.join("memory")).unwrap_or(&f).display().to_string();
            add_file(&mut entries, Scope::ProjectMemory, &group, f, name);
        }

        if let Some(cwd) = &cwd {
            let root = PathBuf::from(cwd);
            if repos_seen.insert(root.clone()) {
                for rel in ["CLAUDE.md", ".claude/CLAUDE.md", "CLAUDE.local.md", "AGENTS.md"] {
                    add_file(&mut entries, Scope::Repo, &group, root.join(rel), rel.into());
                }
                let rules = root.join(".claude/rules");
                for f in md_files_recursive(&rules) {
                    let name = format!(".claude/rules/{}", f.strip_prefix(&rules).unwrap_or(&f).display());
                    add_file(&mut entries, Scope::Repo, &group, f, name);
                }
            }
        }

        for (path, modified) in sessions {
            let id = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
            let title = first_prompt(&path).unwrap_or_else(|| id.clone());
            let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            entries.push(Entry {
                scope: Scope::Session,
                group: group.clone(),
                name: title,
                path,
                size,
                modified,
            });
        }
    }

    entries.sort_by_key(|e| (e.scope as u8, e.group.clone()));
    Install { dir: dir.to_path_buf(), from_env, entries }
}

fn add_file(entries: &mut Vec<Entry>, scope: Scope, group: &str, path: PathBuf, name: String) {
    if let Ok(m) = fs::metadata(&path) {
        if m.is_file() {
            entries.push(Entry {
                scope,
                group: group.to_string(),
                name,
                size: m.len(),
                modified: m.modified().ok(),
                path,
            });
        }
    }
}

fn md_files_recursive(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "md") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// Top-level transcripts only; subagent logs live in nested dirs.
fn session_files(pdir: &Path) -> Vec<(PathBuf, Option<SystemTime>)> {
    let mut v: Vec<_> = fs::read_dir(pdir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .map(|p| {
            let m = fs::metadata(&p).and_then(|m| m.modified()).ok();
            (p, m)
        })
        .collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    v
}

fn project_cwd(jsonl: &Path) -> Option<String> {
    let f = fs::File::open(jsonl).ok()?;
    BufReader::new(f).lines().take(60).flatten().find_map(|l| {
        let v: Value = serde_json::from_str(&l).ok()?;
        v.get("cwd")?.as_str().map(str::to_owned)
    })
}

fn message_text(v: &Value) -> Option<(String, String)> {
    let kind = v.get("type")?.as_str()?;
    if kind != "user" && kind != "assistant" {
        return None;
    }
    if v.get("isMeta").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let content = v.get("message")?.get("content")?;
    let text = match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => return None,
    };
    let text = text.trim().to_string();
    (!text.is_empty()).then(|| (kind.to_string(), text))
}

fn first_prompt(jsonl: &Path) -> Option<String> {
    let f = fs::File::open(jsonl).ok()?;
    BufReader::new(f).lines().take(200).flatten().find_map(|l| {
        let v: Value = serde_json::from_str(&l).ok()?;
        let (kind, text) = message_text(&v)?;
        // Slash-command and hook wrappers are XML-ish, not the user's own words.
        (kind == "user" && !text.starts_with('<')).then(|| text.lines().next().unwrap_or("").chars().take(80).collect())
    })
}

pub fn load(entry: &Entry) -> String {
    if entry.scope == Scope::Session {
        return render_session(&entry.path);
    }
    read_capped(&entry.path)
}

fn read_capped(path: &Path) -> String {
    let mut buf = Vec::new();
    match fs::File::open(path).and_then(|f| f.take(MAX_READ).read_to_end(&mut buf)) {
        Ok(_) => String::from_utf8_lossy(&buf).into_owned(),
        Err(e) => format!("cannot read {}: {e}", path.display()),
    }
}

fn render_session(path: &Path) -> String {
    let Ok(f) = fs::File::open(path) else {
        return format!("cannot read {}", path.display());
    };
    let mut out = String::new();
    for line in BufReader::new(f.take(MAX_READ)).lines().flatten() {
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        let Some((role, text)) = message_text(&v) else { continue };
        let ts = v.get("timestamp").and_then(Value::as_str).unwrap_or("");
        let clipped: String = text.chars().take(1200).collect();
        out.push_str(&format!("## {role}  {ts}\n{clipped}\n\n"));
        if out.len() > MAX_RENDER {
            out.push_str("… truncated\n");
            break;
        }
    }
    if out.is_empty() {
        "(no user/assistant text found)".into()
    } else {
        out
    }
}
