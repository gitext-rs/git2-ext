//! Higher-level git operations
//!
//! These are closer to what you expect to see for porcelain commands, rather than just plumbing.
//! They serve as both examples on how to use `git2` but also should be usable in some limited
//! subset of cases.

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
    let base_id = repo.merge_base(head_id, cherry_id).unwrap_or(cherry_id);
    if base_id == head_id {
        // Already on top of the intended base
        return Ok(cherry_id);
    }

    let base_ann_commit = repo.find_annotated_commit(base_id)?;
    let head_ann_commit = repo.find_annotated_commit(head_id)?;
    let cherry_ann_commit = repo.find_annotated_commit(cherry_id)?;
    let cherry_commit = repo.find_commit(cherry_id)?;
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
                        .map(|c| bytes2path(&c.path))
                        .or_else(|| conflict.their.as_ref().map(|c| bytes2path(&c.path)))
                        .or_else(|| conflict.ancestor.as_ref().map(|c| bytes2path(&c.path)))
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
                    .map(|c| bytes2path(&c.path))
                    .or_else(|| conflict.their.as_ref().map(|c| bytes2path(&c.path)))
                    .or_else(|| conflict.ancestor.as_ref().map(|c| bytes2path(&c.path)))
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
    let new_id = repo.commit(
        None,
        &into_commit.author(),
        &into_commit.committer(),
        into_commit.message().unwrap(),
        &result_tree,
        onto_commits,
    )?;
    Ok(new_id)
}

// From git2 crate
#[cfg(unix)]
fn bytes2path(b: &[u8]) -> &std::path::Path {
    use std::os::unix::prelude::*;
    std::path::Path::new(std::ffi::OsStr::from_bytes(b))
}

// From git2 crate
#[cfg(windows)]
fn bytes2path(b: &[u8]) -> &std::path::Path {
    use std::str;
    std::path::Path::new(str::from_utf8(b).unwrap())
}
