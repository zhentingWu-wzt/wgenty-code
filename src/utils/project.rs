//! Project utilities

use std::path::{Path, PathBuf};

/// Read WGENTY.md sections from project root (split by `---`).
pub fn read_wgenty_md_sections(project_root: &Path) -> Vec<String> {
    read_md_sections(project_root, "WGENTY.md")
}

/// Read AGENTS.md sections from project root (split by `---`).
pub fn read_agents_md_sections(project_root: &Path) -> Vec<String> {
    read_md_sections(project_root, "AGENTS.md")
}

/// Read user-global instructions at `~/.wgenty-code/WGENTY.md`.
///
/// Returns `None` if the home directory cannot be resolved, the file does not
/// exist, cannot be read, or is empty.
pub fn read_user_global_instructions() -> Option<(PathBuf, String)> {
    let home = dirs::home_dir()?;
    read_user_global_instructions_from(&home)
}

/// Testable variant of [`read_user_global_instructions`] that accepts an
/// explicit home directory (avoids polluting the real `HOME` env var in tests).
fn read_user_global_instructions_from(home: &Path) -> Option<(PathBuf, String)> {
    let path = home.join(".wgenty-code").join("WGENTY.md");
    let content = std::fs::read_to_string(&path).ok()?;
    if content.is_empty() {
        None
    } else {
        Some((path, content))
    }
}

fn read_md_sections(root: &Path, filename: &str) -> Vec<String> {
    let path = root.join(filename);
    if !path.exists() {
        return Vec::new();
    }
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    content
        .split("\n---\n")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Initialize a new project
pub fn init_project(name: &str) -> anyhow::Result<()> {
    let project_dir = PathBuf::from(name);

    // Create project directory
    std::fs::create_dir_all(&project_dir)?;

    // Create basic structure
    std::fs::create_dir_all(project_dir.join("src"))?;

    // Create CLAUDE.md file
    let claude_md = format!(
        "# {}\n\nThis project was initialized with Wgenty Code.\n\n## Structure\n\n- `src/` - Source code\n- `CLAUDE.md` - Project documentation for Claude\n\n## Getting Started\n\nStart coding with Wgenty Code!\n",
        name
    );
    std::fs::write(project_dir.join("CLAUDE.md"), claude_md)?;

    // Create .gitignore
    let gitignore = "target/\n*.log\n.env\n";
    std::fs::write(project_dir.join(".gitignore"), gitignore)?;

    println!("Created project structure:");
    println!("  {}/", name);
    println!("    src/");
    println!("    CLAUDE.md");
    println!("    .gitignore");

    Ok(())
}

/// Detect project type
pub fn detect_project_type(path: &std::path::Path) -> ProjectType {
    // Check for various project markers
    if path.join("Cargo.toml").exists() {
        ProjectType::Rust
    } else if path.join("package.json").exists() {
        ProjectType::JavaScript
    } else if path.join("go.mod").exists() {
        ProjectType::Go
    } else if path.join("pyproject.toml").exists() || path.join("setup.py").exists() {
        ProjectType::Python
    } else if path.join("CMakeLists.txt").exists() {
        ProjectType::Cpp
    } else {
        ProjectType::Unknown
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectType {
    Rust,
    JavaScript,
    Python,
    Go,
    Cpp,
    Unknown,
}

impl std::fmt::Display for ProjectType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProjectType::Rust => write!(f, "Rust"),
            ProjectType::JavaScript => write!(f, "JavaScript/TypeScript"),
            ProjectType::Python => write!(f, "Python"),
            ProjectType::Go => write!(f, "Go"),
            ProjectType::Cpp => write!(f, "C/C++"),
            ProjectType::Unknown => write!(f, "Unknown"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn user_instructions_present() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join(".wgenty-code");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("WGENTY.md");
        std::fs::write(&file, "hello").unwrap();

        let got = read_user_global_instructions_from(tmp.path());
        assert_eq!(got, Some((file, "hello".to_string())));
    }

    #[test]
    fn user_instructions_missing_returns_none() {
        let tmp = TempDir::new().unwrap();
        assert!(read_user_global_instructions_from(tmp.path()).is_none());
    }

    #[test]
    fn user_instructions_empty_returns_none() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join(".wgenty-code");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("WGENTY.md"), "").unwrap();

        assert!(read_user_global_instructions_from(tmp.path()).is_none());
    }
}
