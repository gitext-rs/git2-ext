//! Higher-level git operations
//!
//! These are closer to what you expect to see for porcelain commands, rather than just plumbing.
//! They serve as both examples on how to use `git2` but also should be usable in some limited
//! subset of cases.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

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

/// This function is a hot code path. Do not annotate with `#[instrument]`, and
/// be mindful of performance/memory allocations.
fn get_changed_paths_between_trees_internal(
    repo: &git2::Repository,
    acc: &mut Vec<Vec<PathBuf>>,
    current_path: &[PathBuf],
    lhs: Option<&git2::Tree>,
    rhs: Option<&git2::Tree>,
) -> ChangedPathsResult<()> {
    let lhs_entries: Vec<_> = lhs.map(|tree| tree.iter().collect()).unwrap_or_default();
    let lhs_entries: HashMap<&[u8], &git2::TreeEntry> = lhs_entries
        .iter()
        .map(|entry| (entry.name_bytes(), entry))
        .collect();

    let rhs_entries: Vec<_> = rhs.map(|tree| tree.iter().collect()).unwrap_or_default();
    let rhs_entries: HashMap<&[u8], &git2::TreeEntry> = rhs_entries
        .iter()
        .map(|entry| (entry.name_bytes(), entry))
        .collect();

    let all_entry_names: HashSet<&[u8]> = lhs_entries
        .keys()
        .chain(rhs_entries.keys())
        .cloned()
        .collect();
    let entries: HashMap<&[u8], (Option<&git2::TreeEntry>, Option<&git2::TreeEntry>)> =
        all_entry_names
            .into_iter()
            .map(|entry_name| {
                (
                    entry_name,
                    (
                        lhs_entries.get(entry_name).copied(),
                        rhs_entries.get(entry_name).copied(),
                    ),
                )
            })
            .collect();

    for (entry_name, (lhs_entry, rhs_entry)) in entries {
        enum ClassifiedEntry {
            Absent,
            NotATree(git2::Oid, i32),
            Tree(git2::Oid, i32),
        }

        fn classify_entry(entry: Option<&git2::TreeEntry>) -> ChangedPathsResult<ClassifiedEntry> {
            let entry = match entry {
                Some(entry) => entry,
                None => return Ok(ClassifiedEntry::Absent),
            };

            let file_mode = entry.filemode_raw();
            match entry.kind() {
                Some(git2::ObjectType::Tree) => Ok(ClassifiedEntry::Tree(entry.id(), file_mode)),
                _ => Ok(ClassifiedEntry::NotATree(entry.id(), file_mode)),
            }
        }

        let get_tree = |oid| match repo.find_tree(oid) {
            Ok(tree) => Ok(tree),
            Err(err) => Err(ChangedPathsError::TreeLookupFailure { source: err, oid }),
        };

        let full_entry_path = {
            let entry_name = match std::str::from_utf8(entry_name) {
                Ok(entry_name) => entry_name,
                Err(_) => continue,
            };
            let mut full_entry_path = current_path.to_vec();
            full_entry_path.push(PathBuf::from(entry_name));
            full_entry_path
        };
        match (classify_entry(lhs_entry)?, classify_entry(rhs_entry)?) {
            (ClassifiedEntry::Absent, ClassifiedEntry::Absent) => {
                // Shouldn't happen, but there's no issue here.
            }

            (
                ClassifiedEntry::NotATree(lhs_oid, lhs_file_mode),
                ClassifiedEntry::NotATree(rhs_oid, rhs_file_mode),
            ) => {
                if lhs_oid == rhs_oid && lhs_file_mode == rhs_file_mode {
                    // Unchanged file, do nothing.
                } else {
                    // Changed file.
                    acc.push(full_entry_path);
                }
            }

            (ClassifiedEntry::Absent, ClassifiedEntry::NotATree(_, _))
            | (ClassifiedEntry::NotATree(_, _), ClassifiedEntry::Absent) => {
                // Added, removed, or changed file.
                acc.push(full_entry_path);
            }

            (ClassifiedEntry::Absent, ClassifiedEntry::Tree(tree_oid, _))
            | (ClassifiedEntry::Tree(tree_oid, _), ClassifiedEntry::Absent) => {
                // A directory was added or removed. Add all entries from that
                // directory.
                let tree = get_tree(tree_oid)?;
                get_changed_paths_between_trees_internal(
                    repo,
                    acc,
                    &full_entry_path,
                    Some(&tree),
                    None,
                )?;
            }

            (ClassifiedEntry::NotATree(_, _), ClassifiedEntry::Tree(tree_oid, _))
            | (ClassifiedEntry::Tree(tree_oid, _), ClassifiedEntry::NotATree(_, _)) => {
                // A file was changed into a directory. Add both the file and
                // all subdirectory entries as changed entries.
                let tree = get_tree(tree_oid)?;
                get_changed_paths_between_trees_internal(
                    repo,
                    acc,
                    &full_entry_path,
                    Some(&tree),
                    None,
                )?;
                acc.push(full_entry_path);
            }

            (
                ClassifiedEntry::Tree(lhs_tree_oid, lhs_file_mode),
                ClassifiedEntry::Tree(rhs_tree_oid, rhs_file_mode),
            ) => {
                match (
                    (lhs_tree_oid == rhs_tree_oid),
                    // Note that there should only be one possible file mode for
                    // an entry which points to a tree, but it's possible that
                    // some extra non-meaningful bits are set. Should we report
                    // a change in that case? This code takes the conservative
                    // approach and reports a change.
                    (lhs_file_mode == rhs_file_mode),
                ) {
                    (true, true) => {
                        // Unchanged entry, do nothing.
                    }

                    (true, false) => {
                        // Only the directory changed, but none of its contents.
                        acc.push(full_entry_path);
                    }

                    (false, true) => {
                        let lhs_tree = get_tree(lhs_tree_oid)?;
                        let rhs_tree = get_tree(rhs_tree_oid)?;

                        // Only include the files changed in the subtrees, and
                        // not the directory itself.
                        get_changed_paths_between_trees_internal(
                            repo,
                            acc,
                            &full_entry_path,
                            Some(&lhs_tree),
                            Some(&rhs_tree),
                        )?;
                    }

                    (false, false) => {
                        let lhs_tree = get_tree(lhs_tree_oid)?;
                        let rhs_tree = get_tree(rhs_tree_oid)?;

                        get_changed_paths_between_trees_internal(
                            repo,
                            acc,
                            &full_entry_path,
                            Some(&lhs_tree),
                            Some(&rhs_tree),
                        )?;
                        acc.push(full_entry_path);
                    }
                }
            }
        }
    }

    Ok(())
}

#[derive(Debug, PartialEq)]
pub enum ChangedPathsError {
    /// An error occurred when trying to look up a tree by OID.
    TreeLookupFailure { source: git2::Error, oid: git2::Oid },
}

pub type ChangedPathsResult<T> = Result<T, ChangedPathsError>;

/// Calculate which paths have changed between two trees more quickly than
/// libgit2. See https://github.com/libgit2/libgit2/issues/6036 for more
/// discussion.
///
/// The libgit2 implementation works by iterating both trees recursively and
/// comparing them, which is O(n) in the size of the trees. This implementation
/// works by mutually traversing both trees and stopping early for subtrees
/// which are equal, which is O(n) in the number of *changes* instead.
pub fn get_changed_paths_between_trees_fast(
    repo: &git2::Repository,
    lhs: Option<&git2::Tree>,
    rhs: Option<&git2::Tree>,
) -> ChangedPathsResult<HashSet<PathBuf>> {
    let mut acc = Vec::new();
    get_changed_paths_between_trees_internal(repo, &mut acc, &Vec::new(), lhs, rhs)?;
    let changed_paths: HashSet<PathBuf> = acc.into_iter().map(PathBuf::from_iter).collect();
    Ok(changed_paths)
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

/// Reword `head_id`s commit
pub fn reword(
    repo: &git2::Repository,
    head_id: git2::Oid,
    msg: &str,
) -> Result<git2::Oid, git2::Error> {
    let old_commit = repo.find_commit(head_id)?;
    let parents = old_commit.parents().collect::<Vec<_>>();
    let parents = parents.iter().collect::<Vec<_>>();
    let tree = repo.find_tree(old_commit.tree_id())?;
    let new_id = repo.commit(
        None,
        &old_commit.author(),
        &old_commit.committer(),
        msg,
        &tree,
        &parents,
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
