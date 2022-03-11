use std::io::Write;

#[derive(Clone, Debug)]
pub struct Hooks {
    root: std::path::PathBuf,
}

impl Hooks {
    pub fn new(hook_root: impl Into<std::path::PathBuf>) -> Self {
        Self {
            root: hook_root.into(),
        }
    }

    pub fn with_repo(repo: &git2::Repository) -> Result<Self, git2::Error> {
        let config = repo.config()?;
        let root = config
            .get_path("core.hooksPath")
            .unwrap_or_else(|_| repo.path().join("hooks"));
        Ok(Self::new(root))
    }

    pub fn root(&self) -> &std::path::Path {
        &self.root
    }

    pub fn run_hook(
        &self,
        repo: &git2::Repository,
        name: &str,
        args: &[&str],
        stdin: Option<&[u8]>,
        env: &[(&str, &str)],
    ) -> Result<i32, std::io::Error> {
        let hook_path = self.root().join(name);
        if !hook_path.exists() {
            return Ok(0);
        }

        let path = {
            let mut path_components: Vec<std::path::PathBuf> =
                vec![std::fs::canonicalize(self.root())?];
            if let Some(path) = std::env::var_os(std::ffi::OsStr::new("PATH")) {
                path_components.extend(std::env::split_paths(&path));
            }
            std::env::join_paths(path_components)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?
        };

        let sh_path = crate::utils::git_sh().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "No `sh` for running hooks")
        })?;

        // From `githooks(5)`:
        // > Before Git invokes a hook, it changes its working directory to either $GIT_DIR in a bare
        // > repository or the root of the working tree in a non-bare repository. An exception are
        // > hooks triggered during a push (pre-receive, update, post-receive, post-update,
        // > push-to-checkout) which are always executed in $GIT_DIR.
        let cwd = if PUSH_HOOKS.contains(&name) {
            repo.path()
        } else {
            repo.workdir().unwrap_or_else(|| repo.path())
        };

        let mut cmd = std::process::Command::new(sh_path);
        cmd.arg("-c")
            .arg(format!("{} \"$@\"", name))
            .arg(name) // "$@" expands "$1" "$2" "$3" ... but we also must specify $0.
            .args(args)
            .env("PATH", path)
            .current_dir(cwd)
            .stdin(std::process::Stdio::piped());
        for (key, value) in env.iter().copied() {
            cmd.env(key, value);
        }
        let mut process = cmd.spawn()?;
        if let Some(stdin) = stdin {
            process.stdin.as_mut().unwrap().write_all(stdin)?;
        }
        let exit = process.wait()?;

        const SIGNAL_EXIT_CODE: i32 = 1;
        Ok(exit.code().unwrap_or(SIGNAL_EXIT_CODE))
    }
}

const PUSH_HOOKS: &[&str] = &[
    "pre-receive",
    "update",
    "post-receive",
    "post-update",
    "push-to-checkout",
];
