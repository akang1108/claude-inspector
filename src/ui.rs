use std::time::SystemTime;

use anyhow::Result;
use ratatui::{
    DefaultTerminal,
    crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Tabs, Wrap},
};

use crate::scan::{self, Entry, Install, Scope};

enum Row {
    Group(String),
    Item(usize),
}

struct App {
    installs: Vec<Install>,
    cur: usize,
    rows: Vec<Row>,
    list: ListState,
    filter: String,
    typing: bool,
    scope: Scope,
    preview: String,
    scroll: u16,
}

pub fn run(installs: Vec<Install>) -> Result<()> {
    let mut app = App {
        installs,
        cur: 0,
        rows: Vec::new(),
        list: ListState::default(),
        filter: String::new(),
        typing: false,
        scope: Scope::Global,
        preview: String::new(),
        scroll: 0,
    };
    app.rebuild();
    let mut term = ratatui::init();
    let res = app.event_loop(&mut term);
    ratatui::restore();
    res
}

impl App {
    fn entries(&self) -> &[Entry] {
        &self.installs[self.cur].entries
    }

    fn rebuild(&mut self) {
        let f = self.filter.to_lowercase();
        let s = self.scope;
        let mut rows = Vec::new();
        let mut last: Option<String> = None;
        for (i, e) in self.entries().iter().enumerate() {
            let hit = f.is_empty()
                || e.name.to_lowercase().contains(&f)
                || e.group.to_lowercase().contains(&f)
                || e.path.to_string_lossy().to_lowercase().contains(&f);
            if e.scope != s || !hit {
                continue;
            }
            if s.per_project() && last.as_deref() != Some(e.group.as_str()) {
                rows.push(Row::Group(e.group.clone()));
                last = Some(e.group.clone());
            }
            rows.push(Row::Item(i));
        }
        self.rows = rows;
        let first = self.rows.iter().position(|r| matches!(r, Row::Item(_)));
        self.list.select(first);
        self.load_preview();
    }

    fn selected(&self) -> Option<&Entry> {
        match self.rows.get(self.list.selected()?)? {
            Row::Item(i) => self.entries().get(*i),
            _ => None,
        }
    }

    fn load_preview(&mut self) {
        self.scroll = 0;
        self.preview = self.selected().map(scan::load).unwrap_or_default();
    }

    fn step(&mut self, delta: isize) {
        let Some(mut i) = self.list.selected() else { return };
        loop {
            let n = i as isize + delta;
            if n < 0 || n as usize >= self.rows.len() {
                return;
            }
            i = n as usize;
            if matches!(self.rows[i], Row::Item(_)) {
                self.list.select(Some(i));
                self.load_preview();
                return;
            }
        }
    }

    fn jump(&mut self, end: bool) {
        let mut items = self.rows.iter().enumerate().filter(|(_, r)| matches!(r, Row::Item(_)));
        let pick = if end { items.next_back() } else { items.next() };
        if let Some((i, _)) = pick {
            self.list.select(Some(i));
            self.load_preview();
        }
    }

    fn switch_install(&mut self, delta: isize) {
        let n = self.installs.len() as isize;
        self.cur = (self.cur as isize + delta).rem_euclid(n) as usize;
        self.rebuild();
    }

    fn switch_scope(&mut self, delta: isize) {
        let n = Scope::ALL.len() as isize;
        let i = Scope::ALL.iter().position(|&x| x == self.scope).unwrap_or(0) as isize;
        self.scope = Scope::ALL[(i + delta).rem_euclid(n) as usize];
        self.rebuild();
    }

    fn rescan(&mut self) {
        for i in &mut self.installs {
            *i = scan::scan(&i.dir.clone(), i.from_env);
        }
        self.rebuild();
    }

    fn event_loop(&mut self, term: &mut DefaultTerminal) -> Result<()> {
        loop {
            term.draw(|f| self.draw(f))?;
            let Event::Key(k) = event::read()? else { continue };
            if k.kind != KeyEventKind::Press {
                continue;
            }
            if self.typing {
                match k.code {
                    KeyCode::Esc => {
                        self.typing = false;
                        self.filter.clear();
                        self.rebuild();
                    }
                    KeyCode::Enter => self.typing = false,
                    KeyCode::Backspace => {
                        self.filter.pop();
                        self.rebuild();
                    }
                    KeyCode::Char(c) => {
                        self.filter.push(c);
                        self.rebuild();
                    }
                    _ => {}
                }
                continue;
            }
            let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
            match k.code {
                KeyCode::Char('q') => return Ok(()),
                KeyCode::Char('c') if ctrl => return Ok(()),
                KeyCode::Esc if !self.filter.is_empty() => {
                    self.filter.clear();
                    self.rebuild();
                }
                KeyCode::Esc => return Ok(()),
                KeyCode::Down | KeyCode::Char('j') => self.step(1),
                KeyCode::Up | KeyCode::Char('k') => self.step(-1),
                KeyCode::Char('g') | KeyCode::Home => self.jump(false),
                KeyCode::Char('G') | KeyCode::End => self.jump(true),
                KeyCode::PageDown | KeyCode::Char('J') => self.scroll = self.scroll.saturating_add(10),
                KeyCode::PageUp | KeyCode::Char('K') => self.scroll = self.scroll.saturating_sub(10),
                KeyCode::Char('d') if ctrl => self.scroll = self.scroll.saturating_add(10),
                KeyCode::Char('u') if ctrl => self.scroll = self.scroll.saturating_sub(10),
                KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => self.switch_scope(1),
                KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => self.switch_scope(-1),
                KeyCode::Char(']') | KeyCode::Char('i') => self.switch_install(1),
                KeyCode::Char('[') | KeyCode::Char('I') => self.switch_install(-1),
                KeyCode::Char('/') => self.typing = true,
                KeyCode::Char('r') => self.rescan(),
                _ => {}
            }
        }
    }

    fn draw(&mut self, f: &mut ratatui::Frame) {
        let [inst_area, scope_area, body, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .areas(f.area());
        let [left, right] = Layout::horizontal([Constraint::Percentage(42), Constraint::Percentage(58)]).areas(body);

        let mut spans = vec![Span::styled(" Install: ", Style::new().dark_gray())];
        for (n, i) in self.installs.iter().enumerate() {
            let label = format!("{}{}", scan::tilde(&i.dir), if i.from_env { " ●" } else { "" });
            let style = if n == self.cur {
                Style::new().bold().fg(Color::Yellow)
            } else {
                Style::new().dark_gray()
            };
            spans.push(Span::styled(label, style));
            spans.push(Span::raw("   "));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), inst_area);

        let titles: Vec<Line> = Scope::ALL
            .iter()
            .map(|&s| {
                let n = self.entries().iter().filter(|e| e.scope == s).count();
                Line::from(format!("{} ({n})", s.title()))
            })
            .collect();
        let sel = Scope::ALL.iter().position(|&x| x == self.scope).unwrap_or(0);
        f.render_widget(
            Tabs::new(titles)
                .select(sel)
                .block(Block::bordered())
                .highlight_style(Style::new().bold().fg(scope_color(self.scope))),
            scope_area,
        );

        let items: Vec<ListItem> = self.rows.iter().map(|r| self.row_item(r)).collect();
        let mut title = format!(" {} ", self.scope.title());
        if !self.filter.is_empty() || self.typing {
            title.push_str(&format!("· /{}{} ", self.filter, if self.typing { "▏" } else { "" }));
        }
        f.render_stateful_widget(
            List::new(items)
                .block(Block::bordered().title(title))
                .highlight_style(Style::new().bg(Color::DarkGray).bold())
                .highlight_symbol("▶ "),
            left,
            &mut self.list,
        );

        let (ptitle, meta) = match self.selected() {
            Some(e) => (
                format!(" {} ", scan::tilde(&e.path)),
                format!("{} · {} · {}", e.scope.title(), human_size(e.size), ago(e.modified)),
            ),
            None => (" Preview ".into(), String::new()),
        };
        let mut lines = vec![Line::from(meta.dark_gray()), Line::raw("")];
        lines.extend(self.preview.lines().map(style_md));
        f.render_widget(
            Paragraph::new(lines)
                .block(Block::new().borders(Borders::ALL).title(ptitle))
                .wrap(Wrap { trim: false })
                .scroll((self.scroll, 0)),
            right,
        );

        f.render_widget(
            Paragraph::new(" j/k move · J/K scroll · Tab/h/l scope · [ ] install · / filter · r rescan · q quit ").dark_gray(),
            footer,
        );
    }

    fn row_item(&self, r: &Row) -> ListItem<'static> {
        match r {
            Row::Group(g) => ListItem::new(Line::from(Span::styled(
                format!("  {}", scan::tilde(std::path::Path::new(g))),
                Style::new().fg(Color::Blue),
            ))),
            Row::Item(i) => {
                let e = &self.entries()[*i];
                let label = if e.scope == Scope::Session {
                    format!("{:>8}  {}", ago(e.modified), e.name)
                } else {
                    e.name.clone()
                };
                ListItem::new(format!("    {label}"))
            }
        }
    }
}

fn scope_color(s: Scope) -> Color {
    match s {
        Scope::Managed => Color::Red,
        Scope::Global => Color::Magenta,
        Scope::Repo => Color::Green,
        Scope::ProjectMemory => Color::Cyan,
        Scope::Session => Color::Yellow,
    }
}

fn style_md(l: &str) -> Line<'static> {
    let s = l.to_string();
    if s.starts_with('#') {
        Line::from(s.bold().cyan())
    } else if s.starts_with("```") || s == "---" {
        Line::from(s.dark_gray())
    } else {
        Line::from(s)
    }
}

pub fn human_size(b: u64) -> String {
    match b {
        0..=1023 => format!("{b} B"),
        1024..=1_048_575 => format!("{:.1} KB", b as f64 / 1024.0),
        _ => format!("{:.1} MB", b as f64 / 1_048_576.0),
    }
}

fn ago(t: Option<SystemTime>) -> String {
    let Some(secs) = t.and_then(|t| SystemTime::now().duration_since(t).ok()).map(|d| d.as_secs()) else {
        return "?".into();
    };
    match secs {
        0..=59 => "now".into(),
        60..=3599 => format!("{}m ago", secs / 60),
        3600..=86399 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86400),
    }
}
