use std::{
    collections::{HashMap, HashSet},
    env, fs, io,
    path::PathBuf,
    process::Command,
    time::UNIX_EPOCH,
};

use anyhow::Result;
use chrono::{Local, TimeZone};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use regex::Regex;

#[cfg(unix)]
use users::{get_group_by_gid, get_user_by_uid};

mod git;
use git::{GitStatus, get_git_status};

/// Main app structure with UI state
struct App {
    current_dir: PathBuf,
    all_entries: Vec<PathBuf>,
    entries: Vec<PathBuf>,
    state: ListState,

    search_mode: bool,
    create_mode: bool,
    goto_mode: bool,
    show_hidden: bool,

    search_query: String,
    create_query: String,
    goto_query: String,

    rename_mode: bool,
    rename_query: String,
    rename_target: Option<PathBuf>,

    marked_delete: HashSet<PathBuf>,
    copy_buffer: Vec<PathBuf>,
    move_buffer: Vec<PathBuf>,
    preview_content: String,

    symlink_mode: bool,
    symlink_query: String,
    symlink_target: Option<PathBuf>,

    cursor_memory: HashMap<PathBuf, usize>,

    show_git: bool,
    git_status: Option<HashMap<PathBuf, GitStatus>>, // may not be under git tracking
}

impl App {
    /// Initialize app with starting directory
    fn new(start_dir: Option<PathBuf>) -> Result<Self> {
        let dir = start_dir.unwrap_or(env::current_dir()?);

        let mut app = Self {
            current_dir: dir,
            all_entries: Vec::new(),
            entries: Vec::new(),
            state: ListState::default(),
            search_mode: false,
            create_mode: false,
            goto_mode: false,
            show_hidden: false,
            search_query: String::new(),
            create_query: String::new(),
            goto_query: String::new(),
            marked_delete: HashSet::new(),
            copy_buffer: Vec::new(),
            move_buffer: Vec::new(),
            preview_content: String::new(),
            rename_mode: false,
            rename_query: String::new(),
            rename_target: None,
            symlink_mode: false,
            symlink_query: String::new(),
            symlink_target: None,
            cursor_memory: HashMap::new(),
            show_git: true,
            git_status: None,
        };

        app.refresh()?; // initial file list
        Ok(app)
    }

    /// Refresh directory entries
    fn refresh(&mut self) -> Result<()> {
        let mut entries: Vec<PathBuf> = match fs::read_dir(&self.current_dir) {
            Ok(read_dir) => read_dir.filter_map(|e| e.ok().map(|e| e.path())).collect(),
            Err(_) => Vec::new(),
        };

        entries.retain(|p| {
            self.show_hidden
                || !p
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .starts_with('.')
        });

        entries.sort_by_key(|p| (!p.is_dir(), p.clone()));

        self.all_entries = entries.clone();
        self.entries = entries;

        if self.entries.is_empty() {
            self.state.select(None);
        } else {
            let remembered = self
                .cursor_memory
                .get(&self.current_dir)
                .copied()
                .unwrap_or(0);

            let clamped = remembered.min(self.entries.len().saturating_sub(1));
            self.state.select(Some(clamped));
            self.state.select(Some(clamped));
        }

        self.update_preview();
        self.git_status = get_git_status(&self.current_dir);
        Ok(())
    }

    /// Get currently selected path
    fn selected_path(&self) -> Option<PathBuf> {
        let index = self.state.selected()?;
        if index < self.entries.len() {
            Some(self.entries[index].clone())
        } else {
            None
        }
    }

    /// Update preview panel
    fn update_preview(&mut self) {
        self.preview_content.clear();

        let Some(path) = self.selected_path() else {
            return;
        };

        let metadata = match fs::metadata(&path) {
            Ok(m) => m,
            Err(e) => {
                self.preview_content = format!("Cannot read metadata: {}", e);
                return;
            }
        };

        // Last modified
        let modified = metadata
            .modified()
            .ok()
            .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
            .map(|d| {
                Local
                    .timestamp_opt(d.as_secs() as i64, 0)
                    .unwrap()
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string()
            })
            .unwrap_or_else(|| "Unknown".to_string());

        // Current time
        let current_time = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

        // Permissions
        let perms = App::get_permissions_string(&metadata);

        // Owner / group (Unix only)
        #[cfg(unix)]
        let owner_group = {
            use std::os::unix::fs::MetadataExt;
            let uid = metadata.uid();
            let gid = metadata.gid();
            let user = get_user_by_uid(uid)
                .map(|u| u.name().to_string_lossy().to_string())
                .unwrap_or(uid.to_string());
            let group = get_group_by_gid(gid)
                .map(|g| g.name().to_string_lossy().to_string())
                .unwrap_or(gid.to_string());
            format!("Owner: {} | Group: {}", user, group)
        };
        #[cfg(windows)]
        let owner_group = "Owner info not supported on Windows".to_string();

        let mut preview = String::new();

        if path.is_dir() {
            let count = fs::read_dir(&path).map(|r| r.count()).unwrap_or(0);
            preview.push_str(&format!(
                "Type: Directory\nEntries: {}\n{}\nPermissions: {}\nLast Modified: {}\nCurrent Time: {}\n{}\n",
                count,
                owner_group,
                perms,
                modified,
                current_time,
                "-".repeat(50)
            ));
        } else {
            let size = metadata.len();
            let file_type = match fs::read_to_string(&path) {
                Ok(_) => "Text File",
                Err(_) => "Binary/Unreadable",
            };
            preview.push_str(&format!(
                "Type: {}\nSize: {} bytes\n{}\nPermissions: {}\nLast Modified: {}\nCurrent Time: {}\n{}\n",
                file_type,
                size,
                owner_group,
                perms,
                modified,
                current_time,
                "-".repeat(50)
            ));

            if file_type == "Text File" {
                if let Ok(content) = fs::read_to_string(&path) {
                    let lines = content.lines().take(50).collect::<Vec<_>>().join("\n");
                    preview.push_str(&lines);
                }
            }
        }

        self.preview_content = preview;
    }
    #[cfg(unix)]
    fn get_permissions_string(metadata: &fs::Metadata) -> String {
        use std::os::unix::fs::PermissionsExt;
        let mode = metadata.permissions().mode();
        format!(
            "{}{}{}{}{}{}{}{}{}",
            if mode & 0o400 != 0 { "r" } else { "-" },
            if mode & 0o200 != 0 { "w" } else { "-" },
            if mode & 0o100 != 0 { "x" } else { "-" },
            if mode & 0o040 != 0 { "r" } else { "-" },
            if mode & 0o020 != 0 { "w" } else { "-" },
            if mode & 0o010 != 0 { "x" } else { "-" },
            if mode & 0o004 != 0 { "r" } else { "-" },
            if mode & 0o002 != 0 { "w" } else { "-" },
            if mode & 0o001 != 0 { "x" } else { "-" },
        )
    }

    #[cfg(windows)]
    fn get_permissions_string(metadata: &fs::Metadata) -> String {
        let p = metadata.permissions();
        if p.readonly() {
            "Read-Only".to_string()
        } else {
            "Read/Write".to_string()
        }
    }

    fn next(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        let i = self.state.selected().unwrap_or(0);
        let next = if i >= self.entries.len() - 1 {
            0
        } else {
            i + 1
        };
        self.state.select(Some(next));
        self.update_preview();
    }

    fn previous(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        let i = self.state.selected().unwrap_or(0);
        let prev = if i == 0 {
            self.entries.len() - 1
        } else {
            i - 1
        };
        self.state.select(Some(prev));
        self.update_preview();
    }

    fn go_parent(&mut self) -> Result<()> {
        if let Some(parent) = self.current_dir.parent() {
            if let Some(index) = self.state.selected() {
                self.cursor_memory.insert(self.current_dir.clone(), index);
            }
            self.current_dir = parent.to_path_buf();
            self.search_mode = false; // get rid of the search bar when enter/leaving a directory
            self.search_query.clear();
            self.refresh()?;
        }
        Ok(())
    }

    fn enter_dir(&mut self, path: PathBuf) -> Result<()> {
        if path.is_dir() {
            if let Some(index) = self.state.selected() {
                self.cursor_memory.insert(self.current_dir.clone(), index);
            }
            match fs::read_dir(&path) {
                Ok(_) => {
                    self.current_dir = path;
                    self.search_mode = false;
                    self.search_query.clear();
                    self.refresh()?;
                }
                Err(e) => {
                    self.preview_content = format!("Cannot enter directory: {}", e);
                }
            }
        }
        Ok(())
    }

    fn filter(&mut self) {
        if self.search_query.is_empty() {
            self.entries = self.all_entries.clone();
        } else if let Ok(regex) = Regex::new(&format!("(?i){}", self.search_query)) {
            // case insensitive file search
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

    fn toggle_delete(&mut self, path: &PathBuf) {
        if self.marked_delete.contains(path) {
            let res = if path.is_dir() {
                fs::remove_dir_all(path)
            } else {
                fs::remove_file(path)
            };

            if let Err(e) = res {
                self.preview_content = format!("Failed to delete {}: {}", path.display(), e);
            } else {
                self.marked_delete.remove(path);
                let _ = self.refresh();
            }
        } else {
            self.marked_delete.insert(path.clone());
        }
    }

    fn unmark_delete(&mut self, path: &PathBuf) {
        self.marked_delete.remove(path);
    }

    fn create_entry(&mut self) -> Result<()> {
        if self.create_query.is_empty() {
            return Ok(());
        }

        let name = self.create_query.trim();
        let path = self.current_dir.join(name);

        if name.ends_with('/') {
            // Directory mode
            fs::create_dir_all(&path)?;
        } else {
            // File mode: ensure parent directories exist
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::File::create(&path)?;
        }

        self.create_query.clear();
        self.create_mode = false;
        let _ = self.refresh();
        Ok(())
    }

    fn mark_copy(&mut self) {
        if let Some(path) = self.selected_path() {
            if self.copy_buffer.contains(&path) {
                self.copy_buffer.retain(|p| p != &path);
            } else {
                self.copy_buffer.push(path);
            }
        }
    }

    fn mark_move(&mut self) {
        if let Some(path) = self.selected_path() {
            if self.move_buffer.contains(&path) {
                self.move_buffer.retain(|p| p != &path);
            } else {
                self.move_buffer.push(path);
            }
        }
    }

    fn paste(&mut self) -> Result<()> {
        for src in &self.copy_buffer {
            let dest = self.current_dir.join(src.file_name().unwrap());
            let _ = self.copy_path(src, &dest);
        }
        self.copy_buffer.clear();

        for src in &self.move_buffer {
            let dest = self.current_dir.join(src.file_name().unwrap());
            let _ = self.move_path(src, &dest);
        }
        self.move_buffer.clear();

        let _ = self.refresh();
        Ok(())
    }

    fn copy_path(&self, src: &PathBuf, dest: &PathBuf) -> Result<()> {
        // earlier bug: copy and paste to same dir truncated the file to 0 bytes
        // Skip if src and dest are the same file
        if fs::canonicalize(src)? == fs::canonicalize(dest).unwrap_or(dest.clone()) {
            return Ok(()); // do nothing
        }

        if src.is_file() {
            fs::copy(src, dest)?;
        } else if src.is_dir() {
            fs::create_dir_all(dest)?;
            for entry in fs::read_dir(src)? {
                let entry = entry?;
                let src_path = entry.path();
                let dest_path = dest.join(entry.file_name());
                self.copy_path(&src_path, &dest_path)?;
            }
        }
        Ok(())
    }

    fn move_path(&self, src: &PathBuf, dest: &PathBuf) -> Result<()> {
        match fs::rename(src, dest) {
            Ok(_) => Ok(()),
            Err(_) => {
                self.copy_path(src, dest)?;
                if src.is_dir() {
                    fs::remove_dir_all(src)?;
                } else {
                    fs::remove_file(src)?;
                }
                Ok(())
            }
        }
    }
}

/// Suspend terminal for external program
fn suspend_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

/// Resume terminal after suspension
fn resume_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    Ok(Terminal::new(CrosstermBackend::new(io::stdout()))?)
}

/// Open file in editor
fn open_in_editor(path: &PathBuf) -> Result<()> {
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());
    let _ = Command::new(editor).arg(path).status();
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
            // Split main layout vertically: top = panels, bottom = input bar
            let vertical = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(1)])
                .split(f.area());

            // Split top area horizontally: left = file list, right = preview
            let horizontal = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
                .split(vertical[0]);

            // Build file list items
            let items: Vec<ListItem> = app
                .entries
                .iter()
                .map(|p| {
                    let name = p.file_name().unwrap_or_default().to_string_lossy();
                    let display = if app.show_git {
                        if let Some(map) = &app.git_status {
                            // normalize path before looking them up
                            let git = if let Ok(canon) = p.canonicalize() {
                                map.get(&canon).map(|s| s.short()).unwrap_or(".")
                            } else {
                                "."
                            };
                            format!("{:2} {}", git, name)
                        } else {
                            name.to_string()
                        }
                    } else {
                        name.to_string()
                    };

                    let mut style = if p.is_dir() {
                        Style::default().fg(Color::Blue)
                    } else {
                        Style::default()
                    };
                    if app.marked_delete.contains(p) {
                        style = style
                            .bg(Color::Red)
                            .fg(Color::Black)
                            .add_modifier(Modifier::BOLD);
                    } else if app.copy_buffer.contains(p) {
                        style = style
                            .fg(Color::Black)
                            .bg(Color::Green)
                            .add_modifier(Modifier::BOLD);
                    } else if app.move_buffer.contains(p) {
                        style = style
                            .bg(Color::Magenta)
                            .fg(Color::Black)
                            .add_modifier(Modifier::BOLD);
                    }
                    if let Some(target) = &app.rename_target {
                        if target == p {
                            style = style
                                .bg(Color::Yellow)
                                .fg(Color::Black)
                                .add_modifier(Modifier::BOLD);
                        }
                    }
                    if let Some(target) = &app.symlink_target {
                        if target == p {
                            style = style
                                .bg(Color::Yellow)
                                .fg(Color::Black)
                                .add_modifier(Modifier::BOLD);
                        }
                    }
                    ListItem::new(display).style(style)
                })
                .collect();

            // Directory name turns green if under git
            let dir_title = if app.git_status.is_some() {
                Span::styled(
                    app.current_dir.to_string_lossy(),
                    Style::default().fg(Color::LightGreen),
                )
            } else {
                Span::raw(app.current_dir.to_string_lossy())
            };

            let left_block = Block::default().borders(Borders::ALL).title(dir_title);

            // Render file list
            let mut list_state = app.state.clone();
            let list = List::new(items)
                .block(left_block)
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
            f.render_stateful_widget(list, horizontal[0], &mut list_state);

            // Render preview panel
            let preview = Paragraph::new(app.preview_content.clone())
                .block(Block::default().borders(Borders::ALL).title("Preview"))
                .wrap(Wrap { trim: false });
            f.render_widget(preview, horizontal[1]);

            // Render input bar at bottom
            let input_text = if app.search_mode {
                format!("/{}", app.search_query)
            } else if app.create_mode {
                format!("n {}", app.create_query)
            } else if app.goto_mode {
                format!("g {}", app.goto_query)
            } else if app.rename_mode {
                format!("Rename: {}", app.rename_query)
            } else if app.symlink_mode {
                format!("Symlink: {}", app.symlink_query)
            } else {
                "".to_string()
            };

            let input_bar = Paragraph::new(input_text);
            f.render_widget(input_bar, vertical[1]);
        })?;

        // Handle input
        if let Event::Key(KeyEvent { code, .. }) = event::read()? {
            match code {
                // ESC cancels all modes
                KeyCode::Esc => {
                    app.search_mode = false;
                    app.create_mode = false;
                    app.goto_mode = false;
                    app.rename_mode = false;
                    app.symlink_mode = false;
                    app.rename_target = None;
                    app.symlink_target = None;
                    app.marked_delete.clear();
                    app.copy_buffer.clear();
                    app.move_buffer.clear();
                    app.search_query.clear();
                    app.create_query.clear();
                    app.goto_query.clear();
                    app.rename_query.clear();
                    app.symlink_query.clear();
                    app.entries = app.all_entries.clone();
                    app.update_preview();
                }

                // Typing while in symlink mode
                KeyCode::Char(c) if app.symlink_mode => {
                    app.symlink_query.push(c);
                }
                KeyCode::Backspace if app.symlink_mode => {
                    app.symlink_query.pop();
                }
                KeyCode::Enter if app.symlink_mode => {
                    if let Some(src) = app.symlink_target.take() {
                        let name = app.symlink_query.trim();
                        if !name.is_empty() {
                            let dest = app.current_dir.join(name);
                            #[cfg(unix)]
                            std::os::unix::fs::symlink(&src, &dest)?;
                            #[cfg(windows)]
                            if src.is_file() {
                                std::os::windows::fs::symlink_file(&src, &dest)?;
                            } else {
                                std::os::windows::fs::symlink_dir(&src, &dest)?;
                            }
                            let _ = app.refresh();
                        }
                    }
                    app.symlink_mode = false;
                    app.symlink_query.clear();
                }

                // Typing while in rename mode
                KeyCode::Char(c) if app.rename_mode => {
                    app.rename_query.push(c);
                }
                KeyCode::Backspace if app.rename_mode => {
                    app.rename_query.pop();
                }
                KeyCode::Enter if app.rename_mode => {
                    if let Some(old_path) = app.rename_target.take() {
                        let new_name = app.rename_query.trim();
                        if !new_name.is_empty() {
                            let new_path = old_path.parent().unwrap().join(new_name);
                            let _ = fs::rename(old_path, new_path);
                            let _ = app.refresh();
                        }
                    }
                    app.rename_mode = false;
                    app.rename_query.clear();
                }

                // Typing while in search mode
                KeyCode::Char(c) if app.search_mode => {
                    app.search_query.push(c);
                    app.filter();
                }
                KeyCode::Backspace if app.search_mode => {
                    app.search_query.pop();
                    app.filter();
                }

                // Typing while in create mode
                KeyCode::Char(c) if app.create_mode => {
                    app.create_query.push(c);
                }
                KeyCode::Backspace if app.create_mode => {
                    app.create_query.pop();
                }
                KeyCode::Enter if app.create_mode => {
                    let _ = app.create_entry();
                }

                // Typing while in goto mode
                KeyCode::Char(c) if app.goto_mode => {
                    app.goto_query.push(c);
                }
                KeyCode::Backspace if app.goto_mode => {
                    app.goto_query.pop();
                }
                KeyCode::Enter if app.goto_mode => {
                    let path_input = app.goto_query.trim();
                    let target_path = if path_input.starts_with("~") {
                        if let Ok(home) = std::env::var("HOME") {
                            PathBuf::from(home).join(&path_input[1..])
                        } else {
                            app.current_dir.clone()
                        }
                    } else {
                        let p = PathBuf::from(path_input);
                        if p.is_absolute() {
                            p
                        } else {
                            app.current_dir.join(p)
                        }
                    };

                    if target_path.is_dir() {
                        app.current_dir = target_path;
                        let _ = app.refresh();
                    } else {
                        app.preview_content = format!("Directory not found: {}", path_input);
                    }

                    app.goto_mode = false;
                    app.goto_query.clear();
                }

                // Main keybindings (only active when not in a mode)
                KeyCode::Char('s') => {
                    if !app.symlink_mode && !app.rename_mode {
                        if let Some(path) = app.selected_path() {
                            app.symlink_mode = true;
                            app.symlink_query.clear();
                            app.symlink_target = Some(path); // remember target like rename
                        }
                    }
                }

                KeyCode::Char(' ') => {
                    if !app.symlink_mode && !app.rename_mode {
                        if !app.symlink_mode && !app.rename_mode {
                            if let Some(path) = app.selected_path() {
                                app.rename_mode = true;
                                app.rename_query.clear();
                                app.rename_target = Some(path); // store the file to rename
                            }
                        }
                    }
                }

                KeyCode::Char('c') => {
                    if !app.symlink_mode && !app.rename_mode {
                        app.mark_copy();
                    }
                }
                KeyCode::Char('m') => {
                    if !app.symlink_mode && !app.rename_mode {
                        app.mark_move();
                    }
                }
                KeyCode::Char('p') => {
                    if !app.symlink_mode && !app.rename_mode {
                        let _ = app.paste();
                    }
                }
                KeyCode::Char('d') => {
                    if !app.symlink_mode && !app.rename_mode {
                        if let Some(path) = app.selected_path() {
                            app.toggle_delete(&path);
                        }
                    }
                }
                KeyCode::Char('r') => {
                    if !app.symlink_mode && !app.rename_mode {
                        if let Some(path) = app.selected_path() {
                            app.unmark_delete(&path);
                        }
                    }
                }

                KeyCode::Char('q') => break,
                KeyCode::Char('/') => {
                    app.search_mode = true;
                    app.search_query.clear();
                }
                KeyCode::Char('n') => {
                    app.create_mode = true;
                    app.create_query.clear();
                }
                KeyCode::Char('g') if !app.search_mode && !app.create_mode => {
                    app.goto_mode = true;
                    app.goto_query.clear();
                }
                KeyCode::Char('.') => {
                    app.show_hidden = !app.show_hidden;
                    let _ = app.refresh();
                }
                KeyCode::Char('?') => {
                    app.show_git = !app.show_git;
                }

                KeyCode::Down => app.next(),
                KeyCode::Up => app.previous(),
                KeyCode::Left => {
                    let _ = app.go_parent();
                }
                KeyCode::Right => {
                    if let Some(path) = app.selected_path() {
                        let _ = app.enter_dir(path);
                    }
                }

                KeyCode::Enter => {
                    if !app.rename_mode && !app.symlink_mode {
                        if let Some(path) = app.selected_path() {
                            if path.is_file() {
                                suspend_terminal(&mut terminal)?;
                                if let Some(index) = app.state.selected() {
                                    // remember file in cursor_memory
                                    app.cursor_memory.insert(app.current_dir.clone(), index);
                                }

                                let _ = open_in_editor(&path); // refresh after exiting editor, better git annotations
                                app.refresh()?;
                                terminal = resume_terminal()?;
                            } else {
                                let _ = app.enter_dir(path);
                            }
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
