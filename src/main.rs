use std::{
    collections::HashSet,
    env,
    fs,
    io,
    path::PathBuf,
    process::Command,
};

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Terminal,
};
use regex::Regex;

struct App {
    current_dir: PathBuf,
    all_entries: Vec<PathBuf>,
    entries: Vec<PathBuf>,
    state: ListState,
    search_mode: bool,
    create_mode: bool,
    show_hidden: bool,
    search_query: String,
    create_query: String,
    marked_delete: HashSet<PathBuf>,
}

impl App {
    fn new(start_dir: Option<PathBuf>) -> Result<Self> {
        let dir = start_dir.unwrap_or(env::current_dir()?);

        let mut app = Self {
            current_dir: dir,
            all_entries: Vec::new(),
            entries: Vec::new(),
            state: ListState::default(),
            search_mode: false,
            create_mode: false,
            show_hidden: false,
            search_query: String::new(),
            create_query: String::new(),
            marked_delete: HashSet::new(),
        };

        app.refresh()?;
        Ok(app)
    }

    fn refresh(&mut self) -> Result<()> {
        let selected_path = self.selected_path();

        let mut entries: Vec<PathBuf> = fs::read_dir(&self.current_dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                self.show_hidden
                    || !p.file_name().unwrap().to_string_lossy().starts_with('.')
            })
            .collect();

        entries.sort_by_key(|p| (!p.is_dir(), p.clone()));

        self.all_entries = entries.clone();
        self.entries = entries;

        if let Some(sel) = selected_path {
            if let Some(pos) = self.entries.iter().position(|p| *p == sel) {
                self.state.select(Some(pos));
            }
        } else if !self.entries.is_empty() {
            self.state.select(Some(0));
        }

        Ok(())
    }

    fn filter(&mut self) {
        if self.search_query.is_empty() {
            self.entries = self.all_entries.clone();
        } else if let Ok(regex) = Regex::new(&self.search_query) {
            self.entries = self
                .all_entries
                .iter()
                .filter(|p| regex.is_match(&p.file_name().unwrap().to_string_lossy()))
                .cloned()
                .collect();
        }

        if !self.entries.is_empty() {
            self.state.select(Some(0));
        }
    }

    fn create_entry(&mut self) -> Result<()> {
        if self.create_query.is_empty() {
            return Ok(());
        }

        let mut path = self.current_dir.clone();
        let name = self.create_query.trim();

        if name.ends_with('/') {
            path.push(name.trim_end_matches('/'));
            fs::create_dir_all(path)?;
        } else {
            path.push(name);
            fs::File::create(path)?;
        }

        self.create_query.clear();
        self.create_mode = false;
        self.refresh()?;
        Ok(())
    }

    fn selected_path(&self) -> Option<PathBuf> {
        self.state.selected().map(|i| self.entries[i].clone())
    }

    fn next(&mut self) {
        if self.entries.is_empty() { return; }
        let i = self.state.selected().unwrap_or(0);
        let next = if i >= self.entries.len() - 1 { 0 } else { i + 1 };
        self.state.select(Some(next));
    }

    fn previous(&mut self) {
        if self.entries.is_empty() { return; }
        let i = self.state.selected().unwrap_or(0);
        let prev = if i == 0 { self.entries.len() - 1 } else { i - 1 };
        self.state.select(Some(prev));
    }

    fn go_parent(&mut self) -> Result<()> {
        if let Some(parent) = self.current_dir.parent() {
            self.current_dir = parent.to_path_buf();
            self.refresh()?;
        }
        Ok(())
    }

    fn enter_dir(&mut self, path: PathBuf) -> Result<()> {
        if path.is_dir() {
            self.current_dir = path;
            self.refresh()?;
        }
        Ok(())
    }

    fn toggle_delete(&mut self, path: &PathBuf) -> Result<()> {
        if self.marked_delete.contains(path) {
            if path.is_dir() {
                fs::remove_dir_all(path)?;
            } else {
                fs::remove_file(path)?;
            }
            self.marked_delete.remove(path);
            self.refresh()?;
        } else {
            self.marked_delete.insert(path.clone());
        }
        Ok(())
    }

    fn is_match(&self, path: &PathBuf) -> bool {
        if self.search_query.is_empty() { return false; }
        if let Ok(regex) = Regex::new(&self.search_query) {
            return regex.is_match(&path.file_name().unwrap().to_string_lossy());
        }
        false
    }
}

fn suspend_terminal(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn resume_terminal() -> Result<Terminal<CrosstermBackend<std::io::Stdout>>> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    Ok(Terminal::new(CrosstermBackend::new(io::stdout()))?)
}

fn open_in_editor(path: &PathBuf) -> Result<()> {
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());
    Command::new(editor).arg(path).status()?;
    Ok(())
}

fn main() -> Result<()> {
    let start_dir = env::args().nth(1).map(PathBuf::from);

    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    let mut app = App::new(start_dir)?;

    loop {
        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(1), Constraint::Length(1)])
                .split(f.area());

            let items: Vec<ListItem> = app.entries.iter().map(|p| {
                let name = p.file_name().unwrap().to_string_lossy();

                let mut style = if p.is_dir() {
                    Style::default().fg(Color::Blue)
                } else {
                    Style::default()
                };

                if app.is_match(p) {
                    style = style.fg(Color::Yellow).add_modifier(Modifier::BOLD);
                }

                if app.marked_delete.contains(p) {
                    style = style.bg(Color::Red).fg(Color::White).add_modifier(Modifier::BOLD);
                }

                ListItem::new(name.to_string()).style(style)
            }).collect();

            let list = List::new(items)
                .block(Block::default().borders(Borders::ALL).title(app.current_dir.to_string_lossy()))
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

            f.render_stateful_widget(list, chunks[0], &mut app.state);

            let input_bar = if app.search_mode {
                Paragraph::new(format!("/{}", app.search_query))
            } else if app.create_mode {
                Paragraph::new(format!("n {}", app.create_query))
            } else {
                Paragraph::new("")
            };

            f.render_widget(input_bar, chunks[1]);

            let footer = Block::default()
                .title("⬆⬇ Move  ➡ Enter  ⬅ Parent  / Search  n New  . Toggle hidden  d Delete  ⏎ Open  ESC Cancel  q Quit")
                .borders(Borders::ALL);

            f.render_widget(footer, chunks[2]);
        })?;

        if let Event::Key(KeyEvent { code, .. }) = event::read()? {
            match code {

                // ===== INPUT MODES FIRST =====

                KeyCode::Esc => {
                    app.search_mode = false;
                    app.create_mode = false;
                    app.search_query.clear();
                    app.create_query.clear();
                    app.entries = app.all_entries.clone();
                }

                KeyCode::Char(c) if app.search_mode => {
                    app.search_query.push(c);
                    app.filter();
                }

                KeyCode::Backspace if app.search_mode => {
                    app.search_query.pop();
                    app.filter();
                }

                KeyCode::Char(c) if app.create_mode => {
                    app.create_query.push(c);
                }

                KeyCode::Backspace if app.create_mode => {
                    app.create_query.pop();
                }

                KeyCode::Enter if app.create_mode => {
                    app.create_entry()?;
                }

                // ===== NORMAL MODE =====

                KeyCode::Char('q') => break,

                KeyCode::Char('/') => {
                    app.search_mode = true;
                    app.search_query.clear();
                }

                KeyCode::Char('n') => {
                    app.create_mode = true;
                    app.create_query.clear();
                }

                KeyCode::Char('.') => {
                    app.show_hidden = !app.show_hidden;
                    app.refresh()?;
                }

                KeyCode::Char('d') => {
                    if let Some(path) = app.selected_path() {
                        app.toggle_delete(&path)?;
                    }
                }

                KeyCode::Down => app.next(),
                KeyCode::Up => app.previous(),
                KeyCode::Left => app.go_parent()?,

                KeyCode::Right => {
                    if let Some(path) = app.selected_path() {
                        app.enter_dir(path)?;
                    }
                }

                KeyCode::Enter => {
                    if let Some(path) = app.selected_path() {
                        if path.is_file() {
                            suspend_terminal(&mut terminal)?;
                            open_in_editor(&path)?;
                            terminal = resume_terminal()?;
                        } else {
                            app.enter_dir(path)?;
                        }
                    }
                }

                _ => {}
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    println!("{}", app.current_dir.display());
    Ok(())
}
