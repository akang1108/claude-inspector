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
    Ancestor,
    Import,
    ProjectMemory,
    Extra,
    Session,
}

impl Scope {
    pub const ALL: [Scope; 8] = [
        Scope::Managed,
        Scope::Global,
        Scope::Repo,
        Scope::Ancestor,
        Scope::Import,
        Scope::ProjectMemory,
        Scope::Extra,
        Scope::Session,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Scope::Managed => "Managed",
            Scope::Global => "Global",
            Scope::Repo => "Repo",
            Scope::Ancestor => "Parent dirs",
            Scope::Import => "Imports",
            Scope::ProjectMemory => "Auto memory",
            Scope::Extra => "Skills & settings",
            Scope::Session => "Sessions",
        }
    }

    pub fn per_project(self) -> bool {
        !matches!(self, Scope::Managed | Scope::Global)
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
        let cwd = sessions.iter().take(5).find_map(|p| project_cwd(p));
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

        if let Some(cwd) = &cwd {
            add_extras(&mut entries, &group, &PathBuf::from(cwd).join(".claude"));
        }

        for path in sessions {
            let id = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
            let meta = fs::metadata(&path).ok();
            entries.push(Entry {
                scope: Scope::Session,
                group: group.clone(),
                name: first_prompt(&path).unwrap_or(id),
                size: meta.as_ref().map_or(0, |m| m.len()),
                modified: meta.and_then(|m| m.modified().ok()),
                path,
            });
        }
    }

    add_extras(&mut entries, &dir.display().to_string(), dir);
    add_ancestors(&mut entries);
    add_imports(&mut entries);

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

/// Newest first; only top-level transcripts, since subagent logs live in nested dirs.
fn session_files(pdir: &Path) -> Vec<PathBuf> {
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
    v.into_iter().map(|(p, _)| p).collect()
}

/// The project dir name is a lossy encoding of the path, so the real cwd comes from the transcript.
fn project_cwd(jsonl: &Path) -> Option<String> {
    let f = fs::File::open(jsonl).ok()?;
    BufReader::new(f).lines().take(60).flatten().find_map(|l| {
        let v: Value = serde_json::from_str(&l).ok()?;
        v.get("cwd")?.as_str().map(str::to_owned)
    })
}

/// Skills, commands, agents, output styles and settings under a `.claude`-style dir.
fn add_extras(entries: &mut Vec<Entry>, group: &str, base: &Path) {
    for rel in ["settings.json", "settings.local.json"] {
        add_file(entries, Scope::Extra, group, base.join(rel), rel.into());
    }
    for sub in ["skills", "commands", "agents", "output-styles"] {
        let root = base.join(sub);
        for f in md_files_recursive(&root) {
            // Skill folders also hold reference docs that are not loaded on their own.
            if sub == "skills" && f.file_name().is_none_or(|n| n != "SKILL.md") {
                continue;
            }
            let name = format!("{sub}/{}", f.strip_prefix(&root).unwrap_or(&f).display());
            add_file(entries, Scope::Extra, group, f, name);
        }
    }
}

/// CLAUDE.md files above a project's cwd are loaded too, and are easy to forget about.
fn add_ancestors(entries: &mut Vec<Entry>) {
    let mut seen: HashSet<PathBuf> = entries.iter().map(|e| e.path.clone()).collect();
    let roots: HashSet<PathBuf> = entries
        .iter()
        .filter(|e| e.scope == Scope::Repo)
        .map(|e| PathBuf::from(&e.group))
        .collect();
    let mut found = Vec::new();
    for root in roots {
        for anc in root.ancestors().skip(1).filter(|a| a.parent().is_some()) {
            for f in ["CLAUDE.md", "CLAUDE.local.md"] {
                let p = anc.join(f);
                if seen.insert(p.clone()) {
                    add_file(&mut found, Scope::Ancestor, &anc.display().to_string(), p, f.into());
                }
            }
        }
    }
    entries.extend(found);
}

/// Follows `@path` imports in every instruction file, so nested includes show up as their own entries.
fn add_imports(entries: &mut Vec<Entry>) {
    let roots: Vec<PathBuf> = entries
        .iter()
        .filter(|e| matches!(e.scope, Scope::Managed | Scope::Global | Scope::Repo | Scope::Ancestor))
        .map(|e| e.path.clone())
        .collect();
    let mut found = Vec::new();
    for root in roots {
        let group = root.display().to_string();
        let mut seen = HashSet::from([root.clone()]);
        collect_imports(&root, &group, 0, &mut seen, &mut found);
    }
    entries.extend(found);
}

fn collect_imports(file: &Path, group: &str, depth: u8, seen: &mut HashSet<PathBuf>, out: &mut Vec<Entry>) {
    if depth >= 5 {
        return;
    }
    let text = read_capped(file);
    let base = file.parent().unwrap_or(Path::new("/"));
    let mut in_fence = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
        }
        if in_fence {
            continue;
        }
        for tok in line.split_whitespace() {
            let Some(raw) = tok.strip_prefix('@') else { continue };
            let raw = raw.trim_end_matches(|c: char| ",.;:)\"'`".contains(c));
            if raw.is_empty() || !(raw.contains('/') || raw.contains('.')) {
                continue;
            }
            let path = match raw.strip_prefix("~/") {
                Some(rest) => home().join(rest),
                None if raw.starts_with('/') => PathBuf::from(raw),
                None => base.join(raw),
            };
            if !seen.insert(path.clone()) {
                continue;
            }
            match fs::metadata(&path) {
                Ok(m) if m.is_file() => {
                    out.push(Entry {
                        scope: Scope::Import,
                        group: group.to_string(),
                        name: format!("@{raw}"),
                        size: m.len(),
                        modified: m.modified().ok(),
                        path: path.clone(),
                    });
                    collect_imports(&path, group, depth + 1, seen, out);
                }
                _ => out.push(Entry {
                    scope: Scope::Import,
                    group: group.to_string(),
                    name: format!("@{raw} (missing)"),
                    size: 0,
                    modified: None,
                    path,
                }),
            }
        }
    }
}

fn message_text(v: &Value) -> Option<(String, String)> {
    let kind = v.get("type")?.as_str()?;
    if kind != "user" && kind != "assistant" {
        return None;
    }
    if v.get("isMeta").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let text = match v.get("message")?.get("content")? {
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
        (kind == "user" && !text.starts_with('<'))
            .then(|| text.lines().next().unwrap_or("").chars().take(80).collect())
    })
}

pub fn load(entry: &Entry) -> String {
    if entry.scope == Scope::Session {
        return render_session(&entry.path);
    }
    read_capped(&entry.path)
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

fn read_capped(path: &Path) -> String {
    let mut buf = Vec::new();
    match fs::File::open(path).and_then(|f| f.take(MAX_READ).read_to_end(&mut buf)) {
        Ok(_) => String::from_utf8_lossy(&buf).into_owned(),
        Err(e) => format!("cannot read {}: {e}", path.display()),
    }
}
