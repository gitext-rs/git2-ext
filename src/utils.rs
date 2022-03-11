/// Path to a shell suitable for running hooks.
pub fn git_sh() -> Option<std::path::PathBuf> {
    let exe_name = if cfg!(target_os = "windows") {
        "bash.exe"
    } else {
        "sh"
    };

    if cfg!(target_os = "windows") {
        // Prefer git-bash since that is how git will normally be running the hooks
        if let Some(path) = find_git_bash() {
            return Some(path);
        }
    }

    which::which(exe_name).ok()
}

fn find_git_bash() -> Option<std::path::PathBuf> {
    // Git is typically installed at C:\Program Files\Git\cmd\git.exe with the cmd\ directory
    // in the path, however git-bash is usually not in PATH and is in bin\ directory:
    let git_path = which::which("git.exe").ok()?;
    let git_dir = git_path.parent()?.parent()?;
    let git_bash = git_dir.join("bin").join("bash.exe");
    git_bash.is_file().then(|| git_bash)
}
