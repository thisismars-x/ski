use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Command,
};

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

// if not under git tracking immediately returns None
pub fn get_git_status(repo_dir: &Path) -> Option<HashMap<PathBuf, GitStatus>> {
    let output = Command::new("git")
        .arg("status")
        .arg("--porcelain")
        .current_dir(repo_dir)
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

        let code = &line[..2];
        let file = line[3..].trim();

        let status = GitStatus::from_code(code);

        let path = repo_dir.join(file);
        map.insert(path, status);
    }

    Some(map)
}
