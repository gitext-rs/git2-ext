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

    pub fn find_hook(&self, _repo: &git2::Repository, name: &str) -> Option<std::path::PathBuf> {
        let mut hook_path = self.root().join(name);
        if is_executable(&hook_path) {
            return Some(hook_path);
        }

        if !std::env::consts::EXE_SUFFIX.is_empty() {
            hook_path.set_extension(std::env::consts::EXE_SUFFIX);
            if is_executable(&hook_path) {
                return Some(hook_path);
            }
        }

        // Technically, we should check `advice.ignoredHook` and warn users if the hook is present
        // but not executable.  Supporting this in the future is why we accept `repo`.

        None
    }

    pub fn run_hook(
        &self,
        repo: &git2::Repository,
        name: &str,
        args: &[&str],
        stdin: Option<&[u8]>,
        env: &[(&str, &str)],
    ) -> Result<i32, std::io::Error> {
        let hook_path = if let Some(hook_path) = self.find_hook(repo, name) {
            hook_path
        } else {
            return Ok(0);
        };
        let bin_name = hook_path
            .file_name()
            .expect("find_hook always returns a bin name")
            .to_str()
            .expect("find_hook always returns a utf-8 bin name");

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
            .arg(format!("{} \"$@\"", bin_name))
            .arg(bin_name) // "$@" expands "$1" "$2" "$3" ... but we also must specify $0.
            .args(args)
            .env("PATH", path)
            .current_dir(cwd)
            // Technically, git maps stdout to stderr when running hooks
            .stdin(std::process::Stdio::piped());
        for (key, value) in env.iter().copied() {
            cmd.env(key, value);
        }
        let mut process = cmd.spawn()?;
        if let Some(stdin) = stdin {
            use std::io::Write;

            process.stdin.as_mut().unwrap().write_all(stdin)?;
        }
        let exit = process.wait()?;

        const SIGNAL_EXIT_CODE: i32 = 1;
        Ok(exit.code().unwrap_or(SIGNAL_EXIT_CODE))
    }

    /// Run `post-rewrite` hook as if called by `git rebase`
    ///
    /// The hook should be run after any automatic note copying (see "notes.rewrite.<command>" in
    /// git-config(1)) has happened, and thus has access to these notes.
    ///
    /// **changed_shas (old, new):**
    /// - For the squash and fixup operation, all commits that were squashed are listed as being rewritten to the squashed commit. This means
    ///   that there will be several lines sharing the same new-sha1.
    /// - The commits are must be listed in the order that they were processed by rebase.
    /// - `git` doesn't include entries for dropped commits
    pub fn run_post_rewrite_rebase(
        &self,
        repo: &git2::Repository,
        changed_oids: &[(git2::Oid, git2::Oid)],
    ) -> Result<(), std::io::Error> {
        let name = "post-rewrite";
        let command = "rebase";
        let args = [command];
        let mut stdin = String::new();
        for (old_oid, new_oid) in changed_oids {
            use std::fmt::Write;
            writeln!(stdin, "{} {}", old_oid, new_oid).expect("Always writeable");
        }

        let code = self.run_hook(repo, name, &args, Some(stdin.as_bytes()), &[])?;
        log::trace!("Hook `{}` failed with code {}", name, code);

        Ok(())
    }

    /// Run `reference-transaction` hook to signal that all reference updates have been queued to the transaction.
    ///
    /// **changed_refs (old, new, name):**
    /// - `name` is the full name of the ref
    /// - `old` is zeroed out when force updating the reference regardless of its current value or
    ///   when the reference is to be created anew
    ///
    /// On success, call either
    /// - `run_reference_transaction_committed`
    /// - `run_reference_transaction_aborted`.
    ///
    /// On failure, the transaction is considered aborted
    pub fn run_reference_transaction_prepare(
        &self,
        repo: &git2::Repository,
        changed_refs: &[(git2::Oid, git2::Oid, &str)],
    ) -> Result<(), std::io::Error> {
        let name = "reference-transaction";
        let state = "prepare";
        let args = [state];
        let mut stdin = String::new();
        for (old_oid, new_oid, ref_name) in changed_refs {
            use std::fmt::Write;
            writeln!(stdin, "{} {} {}", old_oid, new_oid, ref_name).expect("Always writeable");
        }

        let code = self.run_hook(repo, name, &args, Some(stdin.as_bytes()), &[])?;
        if code == 0 {
            Ok(())
        } else {
            log::trace!("Hook `{}` failed with code {}", name, code);
            Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                format!("`{}` hook failed with code {}", name, code),
            ))
        }
    }

    /// Run `reference-transaction` hook to signal that all reference updates have been applied
    ///
    /// **changed_refs (old, new, name):**
    /// - `name` is the full name of the ref
    /// - `old` is zeroed out when force updating the reference regardless of its current value or
    ///   when the reference is to be created anew
    pub fn run_reference_transaction_committed(
        &self,
        repo: &git2::Repository,
        changed_refs: &[(git2::Oid, git2::Oid, &str)],
    ) -> Result<(), std::io::Error> {
        let name = "reference-transaction";
        let state = "committed";
        let args = [state];
        let mut stdin = String::new();
        for (old_oid, new_oid, ref_name) in changed_refs {
            use std::fmt::Write;
            writeln!(stdin, "{} {} {}", old_oid, new_oid, ref_name).expect("Always writeable");
        }

        let code = self.run_hook(repo, name, &args, Some(stdin.as_bytes()), &[])?;
        log::trace!("Hook `{}` failed with code {}", name, code);

        Ok(())
    }

    /// Run `reference-transaction` hook to signal that no changes have been made
    ///
    /// **changed_refs (old, new, name):**
    /// - `name` is the full name of the ref
    /// - `old` is zeroed out when force updating the reference regardless of its current value or
    ///   when the reference is to be created anew
    pub fn run_reference_transaction_aborted(
        &self,
        repo: &git2::Repository,
        changed_refs: &[(git2::Oid, git2::Oid, &str)],
    ) -> Result<(), std::io::Error> {
        let name = "reference-transaction";
        let state = "aborted";
        let args = [state];
        let mut stdin = String::new();
        for (old_oid, new_oid, ref_name) in changed_refs {
            use std::fmt::Write;
            writeln!(stdin, "{} {} {}", old_oid, new_oid, ref_name).expect("Always writeable");
        }

        let code = self.run_hook(repo, name, &args, Some(stdin.as_bytes()), &[])?;
        log::trace!("Hook `{}` failed with code {}", name, code);

        Ok(())
    }
}

const PUSH_HOOKS: &[&str] = &[
    "pre-receive",
    "update",
    "post-receive",
    "post-update",
    "push-to-checkout",
];

#[cfg(unix)]
fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    let metadata = match path.metadata() {
        Ok(metadata) => metadata,
        Err(_) => return false,
    };
    let permissions = metadata.permissions();
    metadata.is_file() && permissions.mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(path: &std::path::Path) -> bool {
    path.is_file()
}
