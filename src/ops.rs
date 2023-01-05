//! Higher-level git operations
//!
//! These are closer to what you expect to see for porcelain commands, rather than just plumbing.
//! They serve as both examples on how to use `git2` but also should be usable in some limited
//! subset of cases.

use bstr::ByteSlice;
use itertools::Itertools;

/// Lookup the commit ID for `HEAD`
pub fn head_id(repo: &git2::Repository) -> Option<git2::Oid> {
    repo.head().ok()?.resolve().ok()?.target()
}

/// Lookup the branch that HEAD points to
pub fn head_branch(repo: &git2::Repository) -> Option<String> {
    repo.head()
        .ok()?
        .resolve()
        .ok()?
        .shorthand()
        .map(String::from)
}

/// Report if the working directory is dirty
pub fn is_dirty(repo: &git2::Repository) -> bool {
    if repo.state() != git2::RepositoryState::Clean {
        log::trace!("Repository status is unclean: {:?}", repo.state());
        return true;
    }

    let status = repo
        .statuses(Some(git2::StatusOptions::new().include_ignored(false)))
        .unwrap();
    if status.is_empty() {
        false
    } else {
        log::trace!(
            "Repository is dirty: {}",
            status
                .iter()
                .flat_map(|s| s.path().map(|s| s.to_owned()))
                .join(", ")
        );
        true
    }
}

/// Cherry pick a commit onto another without touching the working directory
pub fn cherry_pick(
    repo: &git2::Repository,
    head_id: git2::Oid,
    cherry_id: git2::Oid,
) -> Result<git2::Oid, git2::Error> {
    let cherry_commit = repo.find_commit(cherry_id)?;
    let base_id = match cherry_commit.parent_count() {
        0 => cherry_id,
        1 => cherry_commit.parent_id(0)?,
        _ => cherry_commit
            .parent_ids()
            .find(|id| *id == head_id)
            .map(Result::Ok)
            .unwrap_or_else(|| cherry_commit.parent_id(0))?,
    };
    if base_id == head_id {
        // Already on top of the intended base
        return Ok(cherry_id);
    }

    let base_ann_commit = repo.find_annotated_commit(base_id)?;
    let head_ann_commit = repo.find_annotated_commit(head_id)?;
    let cherry_ann_commit = repo.find_annotated_commit(cherry_id)?;
    let mut rebase = repo.rebase(
        Some(&cherry_ann_commit),
        Some(&base_ann_commit),
        Some(&head_ann_commit),
        Some(git2::RebaseOptions::new().inmemory(true)),
    )?;

    let mut tip_id = head_id;
    while let Some(op) = rebase.next() {
        op.map_err(|e| {
            let _ = rebase.abort();
            e
        })?;
        let inmemory_index = rebase.inmemory_index().unwrap();
        if inmemory_index.has_conflicts() {
            let conflicts = inmemory_index
                .conflicts()?
                .map(|conflict| {
                    let conflict = conflict.unwrap();
                    let our_path = conflict
                        .our
                        .as_ref()
                        .map(|c| crate::bytes::bytes2path(&c.path))
                        .or_else(|| {
                            conflict
                                .their
                                .as_ref()
                                .map(|c| crate::bytes::bytes2path(&c.path))
                        })
                        .or_else(|| {
                            conflict
                                .ancestor
                                .as_ref()
                                .map(|c| crate::bytes::bytes2path(&c.path))
                        })
                        .unwrap_or_else(|| std::path::Path::new("<unknown>"));
                    format!("{}", our_path.display())
                })
                .join("\n  ");
            return Err(git2::Error::new(
                git2::ErrorCode::Unmerged,
                git2::ErrorClass::Index,
                format!("cherry-pick conflicts:\n  {}\n", conflicts),
            ));
        }

        let mut sig = repo.signature()?;
        if let (Some(name), Some(email)) = (sig.name(), sig.email()) {
            // For simple rebases, preserve the original commit time
            sig = git2::Signature::new(name, email, &cherry_commit.time())?.to_owned();
        }
        let commit_id = match rebase.commit(None, &sig, None).map_err(|e| {
            let _ = rebase.abort();
            e
        }) {
            Ok(commit_id) => Ok(commit_id),
            Err(err) => {
                if err.class() == git2::ErrorClass::Rebase && err.code() == git2::ErrorCode::Applied
                {
                    log::trace!("Skipping {}, already applied to {}", cherry_id, head_id);
                    return Ok(tip_id);
                }
                Err(err)
            }
        }?;
        tip_id = commit_id;
    }
    rebase.finish(None)?;
    Ok(tip_id)
}

/// Squash `head_id` into `into_id` without touching the working directory
///
/// `into_id`'s author, committer, and message are preserved.
pub fn squash(
    repo: &git2::Repository,
    head_id: git2::Oid,
    into_id: git2::Oid,
    sign: Option<&dyn Sign>,
) -> Result<git2::Oid, git2::Error> {
    // Based on https://www.pygit2.org/recipes/git-cherry-pick.html
    let head_commit = repo.find_commit(head_id)?;
    let head_tree = repo.find_tree(head_commit.tree_id())?;

    let base_commit = if 0 < head_commit.parent_count() {
        head_commit.parent(0)?
    } else {
        head_commit.clone()
    };
    let base_tree = repo.find_tree(base_commit.tree_id())?;

    let into_commit = repo.find_commit(into_id)?;
    let into_tree = repo.find_tree(into_commit.tree_id())?;

    let onto_commit;
    let onto_commits;
    let onto_commits: &[&git2::Commit] = if 0 < into_commit.parent_count() {
        onto_commit = into_commit.parent(0)?;
        onto_commits = [&onto_commit];
        &onto_commits
    } else {
        &[]
    };

    let mut result_index = repo.merge_trees(&base_tree, &into_tree, &head_tree, None)?;
    if result_index.has_conflicts() {
        let conflicts = result_index
            .conflicts()?
            .map(|conflict| {
                let conflict = conflict.unwrap();
                let our_path = conflict
                    .our
                    .as_ref()
                    .map(|c| crate::bytes::bytes2path(&c.path))
                    .or_else(|| {
                        conflict
                            .their
                            .as_ref()
                            .map(|c| crate::bytes::bytes2path(&c.path))
                    })
                    .or_else(|| {
                        conflict
                            .ancestor
                            .as_ref()
                            .map(|c| crate::bytes::bytes2path(&c.path))
                    })
                    .unwrap_or_else(|| std::path::Path::new("<unknown>"));
                format!("{}", our_path.display())
            })
            .join("\n  ");
        return Err(git2::Error::new(
            git2::ErrorCode::Unmerged,
            git2::ErrorClass::Index,
            format!("squash conflicts:\n  {}\n", conflicts),
        ));
    }
    let result_id = result_index.write_tree_to(repo)?;
    let result_tree = repo.find_tree(result_id)?;
    let new_id = commit(
        repo,
        &into_commit.author(),
        &into_commit.committer(),
        into_commit.message().unwrap(),
        &result_tree,
        onto_commits,
        sign,
    )?;
    Ok(new_id)
}

/// Reword `head_id`s commit
pub fn reword(
    repo: &git2::Repository,
    head_id: git2::Oid,
    msg: &str,
    sign: Option<&dyn Sign>,
) -> Result<git2::Oid, git2::Error> {
    let old_commit = repo.find_commit(head_id)?;
    let parents = old_commit.parents().collect::<Vec<_>>();
    let parents = parents.iter().collect::<Vec<_>>();
    let tree = repo.find_tree(old_commit.tree_id())?;
    let new_id = commit(
        repo,
        &old_commit.author(),
        &old_commit.committer(),
        msg,
        &tree,
        &parents,
        sign,
    )?;
    Ok(new_id)
}

/// Commit with signing support
pub fn commit(
    repo: &git2::Repository,
    author: &git2::Signature<'_>,
    committer: &git2::Signature<'_>,
    message: &str,
    tree: &git2::Tree<'_>,
    parents: &[&git2::Commit<'_>],
    sign: Option<&dyn Sign>,
) -> Result<git2::Oid, git2::Error> {
    if let Some(sign) = sign {
        let content = repo.commit_create_buffer(author, committer, message, tree, parents)?;
        let content = std::str::from_utf8(&content).unwrap();
        let signed = sign.sign(content)?;
        repo.commit_signed(content, &signed, None)
    } else {
        repo.commit(None, author, committer, message, tree, parents)
    }
}

/// For signing [commit]s
///
/// See <https://blog.hackeriet.no/signing-git-commits-in-rust/> for an example of what to do.
pub trait Sign {
    fn sign(&self, buffer: &str) -> Result<String, git2::Error>;
}

pub struct UserSign(UserSignInner);

enum UserSignInner {
    Gpg(GpgSign),
    Ssh(SshSign),
}

impl UserSign {
    pub fn from_config(
        repo: &git2::Repository,
        config: &git2::Config,
    ) -> Result<Self, git2::Error> {
        let format = config
            .get_string("gpg.format")
            .unwrap_or_else(|_| "openpgp".to_owned());
        match format.as_str() {
            "openpgp" => {
                let program = config
                    .get_string("gpg.openpgp.program")
                    .or_else(|_| config.get_string("gpg.program"))
                    .unwrap_or_else(|_| "gpg".to_owned());

                let signing_key = config.get_string("user.signingkey").or_else(
                    |_| -> Result<_, git2::Error> {
                        let sig = repo.signature()?;
                        Ok(String::from_utf8_lossy(sig.name_bytes()).into_owned())
                    },
                )?;

                Ok(UserSign(UserSignInner::Gpg(GpgSign::new(
                    program,
                    signing_key,
                ))))
            }
            "x509" => {
                let program = config
                    .get_string("gpg.x509.program")
                    .unwrap_or_else(|_| "gpgsm".to_owned());

                let signing_key = config.get_string("user.signingkey").or_else(
                    |_| -> Result<_, git2::Error> {
                        let sig = repo.signature()?;
                        Ok(String::from_utf8_lossy(sig.name_bytes()).into_owned())
                    },
                )?;

                Ok(UserSign(UserSignInner::Gpg(GpgSign::new(
                    program,
                    signing_key,
                ))))
            }
            "ssh" => {
                let program = config
                    .get_string("gpg.ssh.program")
                    .unwrap_or_else(|_| "ssh-keygen".to_owned());

                let signing_key = config
                    .get_string("user.signingkey")
                    .map(Ok)
                    .unwrap_or_else(|_| -> Result<_, git2::Error> {
                        get_default_ssh_signing_key(config)?.map(Ok).unwrap_or_else(
                            || -> Result<_, git2::Error> {
                                let sig = repo.signature()?;
                                Ok(String::from_utf8_lossy(sig.name_bytes()).into_owned())
                            },
                        )
                    })?;

                Ok(UserSign(UserSignInner::Ssh(SshSign::new(
                    program,
                    signing_key,
                ))))
            }
            _ => Err(git2::Error::new(
                git2::ErrorCode::Invalid,
                git2::ErrorClass::Config,
                format!("invalid valid for gpg.format: {}", format),
            )),
        }
    }
}

impl Sign for UserSign {
    fn sign(&self, buffer: &str) -> Result<String, git2::Error> {
        match &self.0 {
            UserSignInner::Gpg(s) => s.sign(buffer),
            UserSignInner::Ssh(s) => s.sign(buffer),
        }
    }
}

pub struct GpgSign {
    program: String,
    signing_key: String,
}

impl GpgSign {
    pub fn new(program: String, signing_key: String) -> Self {
        Self {
            program,
            signing_key,
        }
    }
}

impl Sign for GpgSign {
    fn sign(&self, buffer: &str) -> Result<String, git2::Error> {
        let output = pipe_command(
            std::process::Command::new(&self.program)
                .arg("--status-fd=2")
                .arg("-bsau")
                .arg(&self.signing_key),
            Some(buffer),
        )
        .map_err(|e| {
            git2::Error::new(
                git2::ErrorCode::GenericError,
                git2::ErrorClass::Os,
                format!("{} failed to sign the data: {}", self.program, e),
            )
        })?;
        if !output.status.success() {
            return Err(git2::Error::new(
                git2::ErrorCode::GenericError,
                git2::ErrorClass::Os,
                format!("{} failed to sign the data", self.program),
            ));
        }
        if output.stderr.find(b"\n[GNUPG:] SIG_CREATED ").is_none() {
            return Err(git2::Error::new(
                git2::ErrorCode::GenericError,
                git2::ErrorClass::Os,
                format!("{} failed to sign the data", self.program),
            ));
        }

        let sig = std::str::from_utf8(&output.stdout).map_err(|e| {
            git2::Error::new(
                git2::ErrorCode::GenericError,
                git2::ErrorClass::Os,
                format!("{} failed to sign the data: {}", self.program, e),
            )
        })?;

        // Strip CR from the line endings, in case we are on Windows.
        let normalized = remove_cr_after(sig);

        Ok(normalized)
    }
}

pub struct SshSign {
    program: String,
    signing_key: String,
}

impl SshSign {
    pub fn new(program: String, signing_key: String) -> Self {
        Self {
            program,
            signing_key,
        }
    }
}

impl Sign for SshSign {
    fn sign(&self, buffer: &str) -> Result<String, git2::Error> {
        let mut literal_key_file = None;
        let ssh_signing_key_file = if let Some(literal_key) = literal_key(&self.signing_key) {
            let temp = tempfile::NamedTempFile::new().map_err(|e| {
                git2::Error::new(
                    git2::ErrorCode::GenericError,
                    git2::ErrorClass::Os,
                    format!("failed writing ssh signing key: {}", e),
                )
            })?;

            std::fs::write(temp.path(), literal_key).map_err(|e| {
                git2::Error::new(
                    git2::ErrorCode::GenericError,
                    git2::ErrorClass::Os,
                    format!("failed writing ssh signing key: {}", e),
                )
            })?;
            let path = temp.path().to_owned();
            literal_key_file = Some(temp);
            path
        } else {
            fn expanduser(path: &str) -> std::path::PathBuf {
                // HACK: Need a cross-platform solution
                std::path::PathBuf::from(path)
            }

            // We assume a file
            expanduser(&self.signing_key)
        };

        let buffer_file = tempfile::NamedTempFile::new().map_err(|e| {
            git2::Error::new(
                git2::ErrorCode::GenericError,
                git2::ErrorClass::Os,
                format!("failed writing buffer: {}", e),
            )
        })?;
        std::fs::write(buffer_file.path(), buffer).map_err(|e| {
            git2::Error::new(
                git2::ErrorCode::GenericError,
                git2::ErrorClass::Os,
                format!("failed writing buffer: {}", e),
            )
        })?;

        let output = pipe_command(
            std::process::Command::new(&self.program)
                .arg("-Y")
                .arg("sign")
                .arg("-n")
                .arg("git")
                .arg("-f")
                .arg(&ssh_signing_key_file)
                .arg(buffer_file.path()),
            Some(buffer),
        )
        .map_err(|e| {
            git2::Error::new(
                git2::ErrorCode::GenericError,
                git2::ErrorClass::Os,
                format!("{} failed to sign the data: {}", self.program, e),
            )
        })?;
        if !output.status.success() {
            if output.stderr.find("usage:").is_some() {
                return Err(git2::Error::new(
                git2::ErrorCode::GenericError,
                git2::ErrorClass::Os,
                "ssh-keygen -Y sign is needed for ssh signing (available in openssh version 8.2p1+)"
            ));
            } else {
                return Err(git2::Error::new(
                    git2::ErrorCode::GenericError,
                    git2::ErrorClass::Os,
                    format!(
                        "{} failed to sign the data: {}",
                        self.program,
                        String::from_utf8_lossy(&output.stderr)
                    ),
                ));
            }
        }

        let mut ssh_signature_filename = buffer_file.path().as_os_str().to_owned();
        ssh_signature_filename.push(".sig");
        let ssh_signature_filename = std::path::PathBuf::from(ssh_signature_filename);
        let sig = std::fs::read_to_string(&ssh_signature_filename).map_err(|e| {
            git2::Error::new(
                git2::ErrorCode::GenericError,
                git2::ErrorClass::Os,
                format!(
                    "failed reading ssh signing data buffer from {}: {}",
                    ssh_signature_filename.display(),
                    e
                ),
            )
        })?;
        // Strip CR from the line endings, in case we are on Windows.
        let normalized = remove_cr_after(&sig);

        buffer_file.close().map_err(|e| {
            git2::Error::new(
                git2::ErrorCode::GenericError,
                git2::ErrorClass::Os,
                format!("failed writing buffer: {}", e),
            )
        })?;
        if let Some(literal_key_file) = literal_key_file {
            literal_key_file.close().map_err(|e| {
                git2::Error::new(
                    git2::ErrorCode::GenericError,
                    git2::ErrorClass::Os,
                    format!("failed writing ssh signing key: {}", e),
                )
            })?;
        }

        Ok(normalized)
    }
}

fn pipe_command(
    cmd: &mut std::process::Command,
    stdin: Option<&str>,
) -> Result<std::process::Output, std::io::Error> {
    use std::io::Write;

    let mut child = cmd
        .stdin(if stdin.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    if let Some(stdin) = stdin {
        let mut stdin_sync = child.stdin.take().expect("stdin is piped");
        write!(stdin_sync, "{}", stdin)?;
    }
    child.wait_with_output()
}

fn remove_cr_after(sig: &str) -> String {
    let mut normalized = String::new();
    for line in sig.lines() {
        normalized.push_str(line);
        normalized.push('\n');
    }
    normalized
}

fn literal_key(signing_key: &str) -> Option<&str> {
    if let Some(literal) = signing_key.strip_prefix("key::") {
        Some(literal)
    } else if signing_key.starts_with("ssh-") {
        Some(signing_key)
    } else {
        None
    }
}

// Returns the first public key from an ssh-agent to use for signing
fn get_default_ssh_signing_key(config: &git2::Config) -> Result<Option<String>, git2::Error> {
    let ssh_default_key_command = config
        .get_string("gpg.ssh.defaultKeyCommand")
        .map_err(|_| {
            git2::Error::new(
                git2::ErrorCode::Invalid,
                git2::ErrorClass::Config,
                "either user.signingkey or gpg.ssh.defaultKeyCommand needs to be configured",
            )
        })?;
    let ssh_default_key_args = shlex::split(&ssh_default_key_command).ok_or_else(|| {
        git2::Error::new(
            git2::ErrorCode::Invalid,
            git2::ErrorClass::Config,
            format!(
                "malformed gpg.ssh.defaultKeyCommand: {}",
                ssh_default_key_command
            ),
        )
    })?;
    if ssh_default_key_args.is_empty() {
        return Err(git2::Error::new(
            git2::ErrorCode::Invalid,
            git2::ErrorClass::Config,
            format!(
                "malformed gpg.ssh.defaultKeyCommand: {}",
                ssh_default_key_command
            ),
        ));
    }

    let Ok(output) = pipe_command(
            std::process::Command::new(&ssh_default_key_args[0])
            .args(&ssh_default_key_args[1..]),
            None,
        ) else {
            return Ok(None);
    };

    let Ok(keys) = std::str::from_utf8(&output.stdout) else {
            return Ok(None);
        };
    let Some((default_key, _)) = keys.split_once('\n') else {
            return Ok(None);
    };
    // We only use `is_literal_ssh_key` here to check validity
    // The prefix will be stripped when the key is used
    if literal_key(default_key).is_none() {
        return Ok(None);
    }

    Ok(Some(default_key.to_owned()))
}
