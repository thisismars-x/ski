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
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
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
    preview_content: String,
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
            preview_content: String::new(),
        };

        app.refresh()?;
        Ok(app)
    }

    fn refresh(&mut self) -> Result<()> {
        let mut entries: Vec<PathBuf> = fs::read_dir(&self.current_dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                self.show_hidden
                    || !p.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with('.')
            })
            .collect();

        entries.sort_by_key(|p| (!p.is_dir(), p.clone()));

        self.all_entries = entries.clone();
        self.entries = entries;

        if self.entries.is_empty() {
            self.state.select(None);
        } else {
            let selected = self.state.selected().unwrap_or(0);
            let clamped = selected.min(self.entries.len() - 1);
            self.state.select(Some(clamped));
        }

        self.update_preview();
        Ok(())
    }

    fn selected_path(&self) -> Option<PathBuf> {
        let index = self.state.selected()?;
        if index < self.entries.len() {
            Some(self.entries[index].clone())
        } else {
            None
        }
    }

    fn update_preview(&mut self) {
        self.preview_content.clear();

        let Some(path) = self.selected_path() else { return };

        if path.is_dir() {
            if let Ok(read) = fs::read_dir(&path) {
                let count = read.count();
                self.preview_content =
                    format!("Directory\n\n{} entries", count);
            }
            return;
        }

        match fs::read_to_string(&path) {
            Ok(content) => {
                self.preview_content = content
                    .lines()
                    .take(100)
                    .collect::<Vec<_>>()
                    .join("\n");
            }
            Err(_) => {
                self.preview_content =
                    "Binary or unreadable file".to_string();
            }
        }
    }

    fn next(&mut self) {
        if self.entries.is_empty() { return; }
        let i = self.state.selected().unwrap_or(0);
        let next = if i >= self.entries.len() - 1 { 0 } else { i + 1 };
        self.state.select(Some(next));
        self.update_preview();
    }

    fn previous(&mut self) {
        if self.entries.is_empty() { return; }
        let i = self.state.selected().unwrap_or(0);
        let prev = if i == 0 { self.entries.len() - 1 } else { i - 1 };
        self.state.select(Some(prev));
        self.update_preview();
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

        if self.entries.is_empty() {
            self.state.select(None);
        } else {
            self.state.select(Some(0));
        }

        self.update_preview();
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
    let editor = std::env::var("EDITOR")
        .unwrap_or_else(|_| "vim".to_string());
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
            let vertical = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(1)])
                .split(f.area());

            let horizontal = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(40),
                    Constraint::Percentage(60),
                ])
                .split(vertical[0]);

            // let items: Vec<ListItem> = app.entries.iter().map(|p| {
            //     let name = p.file_name().unwrap().to_string_lossy();
            //     let style = if p.is_dir() {
            //         Style::default().fg(Color::Blue)
            //     } else {
            //         Style::default()
            //     };
            //     ListItem::new(name.to_string()).style(style)
            // }).collect();

            let items: Vec<ListItem> = app.entries.iter().map(|p| {
                let name = p.file_name().unwrap().to_string_lossy();

                let mut style = if p.is_dir() {
                    Style::default().fg(Color::Blue)
                } else {
                    Style::default()
                };

                // 🔥 If marked for deletion -> make red + bold
                if app.marked_delete.contains(p) {
                    style = Style::default()
                        .fg(Color::Red)
                        .add_modifier(Modifier::BOLD);
                }

                ListItem::new(name.to_string()).style(style)
            }).collect();

            let list = List::new(items)
                .block(Block::default().borders(Borders::ALL).title(app.current_dir.to_string_lossy()))
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

            f.render_stateful_widget(list, horizontal[0], &mut app.state);

            let preview = Paragraph::new(app.preview_content.clone())
                .block(Block::default().borders(Borders::ALL).title("Preview"))
                .wrap(Wrap { trim: false });

            f.render_widget(preview, horizontal[1]);

            let input_bar = if app.search_mode {
                Paragraph::new(format!("/{}", app.search_query))
            } else if app.create_mode {
                Paragraph::new(format!("n {}", app.create_query))
            } else {
                Paragraph::new("")
            };

            f.render_widget(input_bar, vertical[1]);
        })?;

        if let Event::Key(KeyEvent { code, .. }) = event::read()? {
            match code {

                KeyCode::Esc => {
                    app.search_mode = false;
                    app.create_mode = false;
                    app.search_query.clear();
                    app.create_query.clear();
                    app.entries = app.all_entries.clone();
                    app.update_preview();
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

                KeyCode::Char('q') => break,
                KeyCode::Char('/') => { app.search_mode = true; app.search_query.clear(); }
                KeyCode::Char('n') => { app.create_mode = true; app.create_query.clear(); }
                KeyCode::Char('.') => {
                    app.show_hidden = !app.show_hidden;
                    app.refresh()?;
                }

                KeyCode::Char('d') => {
                    if let Some(path) = app.selected_path() {
                        app.toggle_delete(&path)?;
                    }
                }

                KeyCode::Char('r') => {
                    if let Some(path) = app.selected_path() {
                        if app.marked_delete.contains(&path) {
                            // unmark for deletion
                            app.marked_delete.remove(&path);
                        }
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
    Ok(())
}
