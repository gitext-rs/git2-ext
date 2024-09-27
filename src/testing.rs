//! Testing utilities.
//!
//! This is inside `src` rather than `tests` since we use this code in some unit
//! tests.

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::io::Write;
use std::ops::Deref;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use assert_fs::TempDir;
use eyre::Context;
use itertools::Itertools;
use regex::{Captures, Regex};

const DUMMY_NAME: &str = "Testy McTestface";
const DUMMY_EMAIL: &str = "test@example.com";
const DUMMY_DATE: &str = "Wed 29 Oct 12:34:56 2020 PDT";

/// Wrapper around the Git executable, for testing.
#[derive(Clone, Debug)]
pub(crate) struct Git {
    /// The path to the repository on disk. The directory itself must exist,
    /// although it might not have a `.git` folder in it. (Use `Git::init_repo`
    /// to initialize it.)
    pub(crate) repo_path: PathBuf,

    /// The path to the Git executable on disk. This is important since we test
    /// against multiple Git versions.
    pub(crate) path_to_git: PathBuf,
}

/// Options for `Git::init_repo_with_options`.
#[derive(Debug)]
pub(crate) struct GitInitOptions {
    /// If `true`, then `init_repo_with_options` makes an initial commit with
    /// some content.
    pub(crate) make_initial_commit: bool,
}

impl Default for GitInitOptions {
    fn default() -> Self {
        GitInitOptions {
            make_initial_commit: true,
        }
    }
}

/// Path to the `git` executable on disk to be executed.
#[derive(Clone)]
pub(crate) struct GitRunInfo {
    /// The path to the Git executable on disk.
    pub(crate) path_to_git: PathBuf,

    /// The working directory that the Git executable should be run in.
    pub(crate) working_directory: PathBuf,
}

impl std::fmt::Debug for GitRunInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "<GitRunInfo path_to_git={:?} working_directory={:?}>",
            self.path_to_git, self.working_directory
        )
    }
}

/// Options for `Git::run_with_options`.
#[derive(Debug, Default)]
pub(crate) struct GitRunOptions {
    /// The timestamp of the command. Mostly useful for `git commit`. This should
    /// be a number like 0, 1, 2, 3...
    pub(crate) time: isize,

    /// The exit code that `Git` should return.
    pub(crate) expected_exit_code: i32,

    /// The input to write to the child process's stdin.
    pub(crate) input: Option<String>,

    /// Additional environment variables to start the process with.
    pub(crate) env: HashMap<String, String>,
}

/// The parsed version of Git.
#[derive(Debug, PartialEq, PartialOrd, Eq)]
pub(crate) struct GitVersion(pub(crate) isize, pub(crate) isize, pub(crate) isize);

impl std::str::FromStr for GitVersion {
    type Err = eyre::Error;

    fn from_str(output: &str) -> eyre::Result<GitVersion> {
        let output = output.trim();
        let words = output.split(&[' ', '-'][..]).collect::<Vec<&str>>();
        let version_str = match &words.as_slice() {
            [_git, _version, version_str, ..] => version_str,
            _ => eyre::bail!("Could not parse Git version output: {:?}", output),
        };
        match version_str.split('.').collect::<Vec<&str>>().as_slice() {
            [major, minor, patch, ..] => {
                let major = major.parse()?;
                let minor = minor.parse()?;

                // Example version without a real patch number: `2.33.GIT`.
                let patch: isize = patch.parse().unwrap_or_default();

                Ok(GitVersion(major, minor, patch))
            }
            _ => eyre::bail!("Could not parse Git version string: {}", version_str),
        }
    }
}

impl Git {
    /// Constructor.
    pub(crate) fn new(git_run_info: GitRunInfo, repo_path: PathBuf) -> Self {
        let GitRunInfo {
            path_to_git,
            // We pass the repo directory when calling `run`.
            working_directory: _,
        } = git_run_info;
        Git {
            repo_path,
            path_to_git,
        }
    }

    /// Replace dynamic strings in the output, for testing purposes.
    pub(crate) fn preprocess_output(&self, stdout: String) -> eyre::Result<String> {
        let path_to_git = self
            .path_to_git
            .to_str()
            .ok_or_else(|| eyre::eyre!("Could not convert path to Git to string"))?;
        let output = stdout.replace(path_to_git, "<git-executable>");

        // NB: tests which run on Windows are unlikely to succeed due to this
        // `canonicalize` call.
        let repo_path = std::fs::canonicalize(&self.repo_path)?;

        let repo_path = repo_path
            .to_str()
            .ok_or_else(|| eyre::eyre!("Could not convert repo path to string"))?;
        let output = output.replace(repo_path, "<repo-path>");

        // Simulate clearing the terminal line by searching for the
        // appropriate sequences of characters and removing the line
        // preceding them.
        //
        // - `\r`: Interactive progress displays may update the same line
        // multiple times with a carriage return before emitting the final
        // newline.
        // - `\x1B[K`: Window pseudo console may emit EL 'Erase in Line' VT
        // sequences.
        let clear_line_re: Regex = Regex::new(r"(^|\n).*(\r|\x1B\[K)").unwrap();
        let output = clear_line_re
            .replace_all(&output, |captures: &Captures<'_>| {
                // Restore the leading newline, if any.
                captures[1].to_string()
            })
            .into_owned();

        Ok(output)
    }

    /// Get the environment variables needed to run git in the test environment.
    pub(crate) fn get_base_env(&self, time: isize) -> Vec<(OsString, OsString)> {
        // Required for determinism, as these values will be baked into the commit
        // hash.
        let date: OsString = format!("{DUMMY_DATE} -{time:0>2}").into();

        // Fake "editor" which accepts the default contents of any commit
        // messages. Usually, we can set this with `git commit -m`, but we have
        // no such option for things such as `git rebase`, which may call `git
        // commit` later as a part of their execution.
        //
        // ":" is understood by `git` to skip editing.
        let git_editor = OsString::from(":");

        let envs = vec![
            ("GIT_CONFIG_NOSYSTEM", OsString::from("1")),
            ("GIT_AUTHOR_DATE", date.clone()),
            ("GIT_COMMITTER_DATE", date),
            ("GIT_EDITOR", git_editor),
        ];

        envs.into_iter()
            .map(|(key, value)| (OsString::from(key), value))
            .collect()
    }

    fn run_with_options_inner(
        &self,
        args: &[&str],
        options: &GitRunOptions,
    ) -> eyre::Result<(String, String)> {
        let GitRunOptions {
            time,
            expected_exit_code,
            input,
            env,
        } = options;

        let env: BTreeMap<_, _> = self
            .get_base_env(*time)
            .into_iter()
            .chain(
                env.iter()
                    .map(|(k, v)| (OsString::from(k), OsString::from(v))),
            )
            .collect();
        let mut command = Command::new(&self.path_to_git);
        command
            .current_dir(&self.repo_path)
            .args(args)
            .env_clear()
            .envs(&env);

        let result = if let Some(input) = input {
            let mut child = command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?;
            write!(child.stdin.take().unwrap(), "{}", &input)?;
            child.wait_with_output().wrap_err_with(|| {
                format!(
                    "Running git
                    Executable: {:?}
                    Args: {:?}
                    Stdin: {:?}
                    Env: <not shown>",
                    &self.path_to_git, &args, input
                )
            })?
        } else {
            command.output().wrap_err_with(|| {
                format!(
                    "Running git
                    Executable: {:?}
                    Args: {:?}
                    Env: <not shown>",
                    &self.path_to_git, &args
                )
            })?
        };

        let exit_code = result
            .status
            .code()
            .expect("Failed to read exit code from Git process");
        let result = if exit_code != *expected_exit_code {
            eyre::bail!(
                "Git command {:?} {:?} exited with unexpected code {} (expected {})
env:
{:#?}
stdout:
{}
stderr:
{}",
                &self.path_to_git,
                &args,
                exit_code,
                expected_exit_code,
                &env,
                &String::from_utf8_lossy(&result.stdout),
                &String::from_utf8_lossy(&result.stderr),
            )
        } else {
            result
        };
        let stdout = String::from_utf8(result.stdout)?;
        let stdout = self.preprocess_output(stdout)?;
        let stderr = String::from_utf8(result.stderr)?;
        let stderr = self.preprocess_output(stderr)?;
        Ok((stdout, stderr))
    }

    /// Run a Git command.
    pub(crate) fn run_with_options<S: AsRef<str> + std::fmt::Debug>(
        &self,
        args: &[S],
        options: &GitRunOptions,
    ) -> eyre::Result<(String, String)> {
        self.run_with_options_inner(
            args.iter().map(|arg| arg.as_ref()).collect_vec().as_slice(),
            options,
        )
    }

    /// Run a Git command.
    pub(crate) fn run<S: AsRef<str> + std::fmt::Debug>(
        &self,
        args: &[S],
    ) -> eyre::Result<(String, String)> {
        self.run_with_options(args, &Default::default())
    }

    /// Set up a Git repo in the directory and initialize git to work
    /// with it.
    pub(crate) fn init_repo_with_options(&self, options: &GitInitOptions) -> eyre::Result<()> {
        self.run(&["init"])?;
        self.run(&["config", "user.name", DUMMY_NAME])?;
        self.run(&["config", "user.email", DUMMY_EMAIL])?;

        if options.make_initial_commit {
            self.commit_file("initial", 0)?;
        }

        // Disable warnings of the following form on Windows:
        //
        // ```
        // warning: LF will be replaced by CRLF in initial.txt.
        // The file will have its original line endings in your working directory
        // ```
        self.run(&["config", "core.autocrlf", "false"])?;

        Ok(())
    }

    /// Set up a Git repo in the directory and initialize git to work
    /// with it.
    pub(crate) fn init_repo(&self) -> eyre::Result<()> {
        self.init_repo_with_options(&Default::default())
    }

    /// Write the provided contents to the provided file in the repository root.
    pub(crate) fn write_file(&self, name: &str, contents: &str) -> eyre::Result<()> {
        let path = PathBuf::from(name);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(self.repo_path.join(dir))?;
        }
        let file_path = self.repo_path.join(format!("{name}.txt"));
        std::fs::write(file_path, contents)?;
        Ok(())
    }

    /// Commit a file with default contents. The `time` argument is used to set
    /// the commit timestamp, which is factored into the commit hash.
    pub(crate) fn commit_file_with_contents(
        &self,
        name: &str,
        time: isize,
        contents: &str,
    ) -> eyre::Result<git2::Oid> {
        self.write_file(name, contents)?;
        self.run(&["add", "."])?;
        self.run_with_options(
            &["commit", "-m", &format!("create {name}.txt")],
            &GitRunOptions {
                time,
                ..Default::default()
            },
        )?;

        let repo = self.get_repo()?;
        let oid = repo
            .head()?
            .target()
            .expect("Could not find OID for just-created commit");
        Ok(oid)
    }

    /// Commit a file with default contents. The `time` argument is used to set
    /// the commit timestamp, which is factored into the commit hash.
    pub(crate) fn commit_file(&self, name: &str, time: isize) -> eyre::Result<git2::Oid> {
        self.commit_file_with_contents(name, time, &format!("{name} contents\n"))
    }

    /// Get a `Repo` object for this repository.
    pub(crate) fn get_repo(&self) -> eyre::Result<git2::Repository> {
        let repo = git2::Repository::open(&self.repo_path)?;
        Ok(repo)
    }
}

/// Wrapper around a `Git` instance which cleans up the repository once dropped.
pub(crate) struct GitWrapper {
    #[allow(dead_code)]
    repo_dir: TempDir,
    git: Git,
}

impl Deref for GitWrapper {
    type Target = Git;

    fn deref(&self) -> &Self::Target {
        &self.git
    }
}

/// Create a temporary directory for testing and a `Git` instance to use with it.
pub(crate) fn make_git() -> eyre::Result<GitWrapper> {
    let repo_dir = TempDir::new()?;
    let path_to_git = get_path_to_git()?;
    let git_run_info = GitRunInfo {
        path_to_git,
        working_directory: repo_dir.path().to_path_buf(),
    };
    let git = Git::new(git_run_info, repo_dir.path().to_path_buf());
    Ok(GitWrapper { repo_dir, git })
}

fn get_path_to_git() -> eyre::Result<PathBuf> {
    let path = which::which("git")?;
    Ok(path)
}
