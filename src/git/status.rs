use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Command,
};

/// Atomic file state under git control
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
    UpdatedButUnmerged,
    Untracked,
    Unknown(String),
}

impl GitStatus {
    pub fn from_code(code: &str) -> Self {
        match code.trim() {
            "M" | "MM" | "AM" => GitStatus::Modified,
            "A" => GitStatus::Added,
            "D" => GitStatus::Deleted,
            "R" => GitStatus::Renamed,
            "C" => GitStatus::Copied,
            "U" => GitStatus::UpdatedButUnmerged,
            "??" => GitStatus::Untracked,
            other => GitStatus::Unknown(other.to_string()),
        }
    }

    pub fn short(&self) -> &str {
        match self {
            GitStatus::Modified => "M",
            GitStatus::Added => "A",
            GitStatus::Deleted => "D",
            GitStatus::Renamed => "R",
            GitStatus::Copied => "C",
            GitStatus::UpdatedButUnmerged => "U",
            GitStatus::Untracked => "??",
            GitStatus::Unknown(s) => s,
        }
    }
}

/// Find the git repository root
fn git_root(path: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .arg("rev-parse")
        .arg("--show-toplevel")
        .current_dir(path)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Some(PathBuf::from(root))
}

/// Return None if not under git tracking
pub fn get_git_status(current_dir: &Path) -> Option<HashMap<PathBuf, GitStatus>> {
    let repo_root = git_root(current_dir)?;

    let output = Command::new("git")
        .arg("status")
        .arg("--porcelain")
        .current_dir(&repo_root)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let mut map = HashMap::new();
    let stdout = String::from_utf8_lossy(&output.stdout);

    for line in stdout.lines() {
        if line.len() < 4 {
            continue;
        }

        // First two characters = git status code
        let code = &line[..2];

        // Remaining part is the file path
        let raw = line[3..].trim();

        // Handle rename format: "old -> new"
        let file = if raw.contains(" -> ") {
            raw.split(" -> ").last().unwrap()
        } else {
            raw
        };

        let status = GitStatus::from_code(code);

        // Convert to absolute path so it matches your explorer entries
        let path = repo_root.join(file);

        map.insert(path, status);
    }

    Some(map)
}
