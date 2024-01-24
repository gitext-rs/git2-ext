//! Lower-level Tree operations

use std::collections::HashMap;
use std::collections::HashSet;
use std::iter::FromIterator;

use itertools::Itertools;

/// This function is a hot code path. Do not annotate with `#[instrument]`, and
/// be mindful of performance/memory allocations.
fn get_changed_paths_between_trees_internal(
    repo: &git2::Repository,
    acc: &mut Vec<Vec<std::path::PathBuf>>,
    current_path: &[std::path::PathBuf],
    lhs: Option<&git2::Tree>,
    rhs: Option<&git2::Tree>,
) -> Result<(), git2::Error> {
    let lhs_entries = lhs
        .map(|tree| tree.iter().collect_vec())
        .unwrap_or_default();
    let lhs_entries: HashMap<&[u8], &git2::TreeEntry> = lhs_entries
        .iter()
        .map(|entry| (entry.name_bytes(), entry))
        .collect();

    let rhs_entries = rhs
        .map(|tree| tree.iter().collect_vec())
        .unwrap_or_default();
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

        fn classify_entry(entry: Option<&git2::TreeEntry>) -> Result<ClassifiedEntry, git2::Error> {
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

        let get_tree = |oid| repo.find_tree(oid);

        let full_entry_path = || -> Vec<std::path::PathBuf> {
            let entry_path = crate::bytes::bytes2path(entry_name);
            let mut full_entry_path = current_path.to_vec();
            full_entry_path.push(entry_path.to_owned());
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
                    acc.push(full_entry_path());
                }
            }

            (ClassifiedEntry::Absent, ClassifiedEntry::NotATree(_, _))
            | (ClassifiedEntry::NotATree(_, _), ClassifiedEntry::Absent) => {
                // Added, removed, or changed file.
                acc.push(full_entry_path());
            }

            (ClassifiedEntry::Absent, ClassifiedEntry::Tree(tree_oid, _))
            | (ClassifiedEntry::Tree(tree_oid, _), ClassifiedEntry::Absent) => {
                // A directory was added or removed. Add all entries from that
                // directory.
                let full_entry_path = full_entry_path();
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
                let full_entry_path = full_entry_path();
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
                        acc.push(full_entry_path());
                    }

                    (false, true) => {
                        let lhs_tree = get_tree(lhs_tree_oid)?;
                        let rhs_tree = get_tree(rhs_tree_oid)?;

                        // Only include the files changed in the subtrees, and
                        // not the directory itself.
                        get_changed_paths_between_trees_internal(
                            repo,
                            acc,
                            &full_entry_path(),
                            Some(&lhs_tree),
                            Some(&rhs_tree),
                        )?;
                    }

                    (false, false) => {
                        let lhs_tree = get_tree(lhs_tree_oid)?;
                        let rhs_tree = get_tree(rhs_tree_oid)?;
                        let full_entry_path = full_entry_path();

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

pub fn get_changed_paths_between_trees(
    repo: &git2::Repository,
    lhs: Option<&git2::Tree>,
    rhs: Option<&git2::Tree>,
) -> Result<HashSet<std::path::PathBuf>, git2::Error> {
    let mut acc = Vec::new();
    get_changed_paths_between_trees_internal(repo, &mut acc, &Vec::new(), lhs, rhs)?;
    let changed_paths: HashSet<_> = acc.into_iter().map(std::path::PathBuf::from_iter).collect();
    Ok(changed_paths)
}

/// Add the provided entries into the tree.
///
/// If the provided `Tree` is `None`, then this function adds the entries to the
/// empty tree.
///
/// The paths for the provided entries can contain slashes.
///
/// If the value for an entry is `None`, then that element in the tree is
/// removed. If a directory ever becomes empty, then it's removed from its
/// parent directory.
///
/// If a path for a given entry is already present in the provided tree, then
/// that entry is overwritten.
///
/// If a path refers to intermediate directories that don't exist in the
/// provided tree, then those intermediate directories are created.
pub fn rebuild_tree<'r>(
    repo: &'r git2::Repository,
    tree: Option<&git2::Tree<'r>>,
    entries: HashMap<std::path::PathBuf, Option<(git2::Oid, i32)>>,
) -> Result<git2::Oid, git2::Error> {
    let (file_entries, dir_entries) = {
        let mut file_entries: HashMap<std::path::PathBuf, Option<(git2::Oid, i32)>> =
            HashMap::new();
        let mut dir_entries: HashMap<
            std::path::PathBuf,
            HashMap<std::path::PathBuf, Option<(git2::Oid, i32)>>,
        > = HashMap::new();
        for (path, value) in entries {
            match path.components().collect_vec().as_slice() {
                [] => {
                    log::trace!("Empty path when hydrating tree");
                }
                [file_name] => {
                    file_entries.insert(file_name.into(), value);
                }
                components => {
                    let first: std::path::PathBuf = [components[0]].iter().collect();
                    let rest: std::path::PathBuf = components[1..].iter().collect();
                    dir_entries.entry(first).or_default().insert(rest, value);
                }
            }
        }
        (file_entries, dir_entries)
    };

    let mut builder = repo.treebuilder(tree)?;
    for (file_name, file_value) in file_entries {
        match file_value {
            Some((oid, file_mode)) => {
                builder.insert(&file_name, oid, file_mode)?;
            }
            None => {
                remove_entry_if_exists(&mut builder, &file_name)?;
            }
        }
    }

    for (dir_name, dir_value) in dir_entries {
        let existing_dir_entry: Option<git2::Tree<'_>> = match builder.get(&dir_name)? {
            Some(existing_dir_entry)
                if !existing_dir_entry.id().is_zero()
                    && existing_dir_entry.kind() == Some(git2::ObjectType::Tree) =>
            {
                Some(repo.find_tree(existing_dir_entry.id())?)
            }
            _ => None,
        };
        let new_entry_oid = rebuild_tree(repo, existing_dir_entry.as_ref(), dir_value)?;

        let new_entry_tree = repo.find_tree(new_entry_oid)?;
        if new_entry_tree.is_empty() {
            remove_entry_if_exists(&mut builder, &dir_name)?;
        } else {
            builder.insert(&dir_name, new_entry_oid, git2::FileMode::Tree.into())?;
        }
    }

    let tree_oid = builder.write()?;
    Ok(tree_oid)
}

/// `libgit2` raises an error if the entry isn't present, but that's often not
/// an error condition here. We may be referring to a created or deleted path,
/// which wouldn't exist in one of the pre-/post-patch trees.
fn remove_entry_if_exists(
    builder: &mut git2::TreeBuilder,
    name: &std::path::Path,
) -> Result<(), git2::Error> {
    if builder.get(name)?.is_some() {
        builder.remove(name)?;
    }
    Ok(())
}

/// Filter the entries in the provided tree by only keeping the provided paths.
///
/// If a provided path does not appear in the tree at all, then it's ignored.
pub fn filter_tree<'r>(
    repo: &'r git2::Repository,
    tree: &git2::Tree<'r>,
    paths: &[&std::path::Path],
) -> Result<git2::Oid, git2::Error> {
    let entries: HashMap<std::path::PathBuf, Option<(git2::Oid, i32)>> = paths
        .iter()
        .map(|path| -> Result<(std::path::PathBuf, _), git2::Error> {
            let key = path.to_path_buf();
            match tree.get_path(path) {
                Ok(tree_entry) => {
                    let value = Some((tree_entry.id(), tree_entry.filemode()));
                    Ok((key, value))
                }
                Err(err) if err.code() == git2::ErrorCode::NotFound => Ok((key, None)),
                Err(err) => Err(err),
            }
        })
        .try_collect()?;

    rebuild_tree(repo, None, entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::testing::make_git;

    fn dump_tree_entries(tree: &git2::Tree<'_>) -> String {
        tree.iter()
            .map(|entry| format!("{:?} {:?}\n", entry.name().unwrap(), entry.id()))
            .collect()
    }

    #[test]
    fn test_rebuild_tree() -> eyre::Result<()> {
        let git = make_git()?;

        git.init_repo()?;

        git.write_file("foo", "foo")?;
        git.write_file("bar/bar", "bar")?;
        git.write_file("bar/baz", "qux")?;
        git.write_file("xyzzy", "xyzzy")?;
        git.run(&["add", "."])?;
        git.run(&["commit", "-m", "commit"])?;

        let repo = git.get_repo()?;
        let head_oid = repo.head()?.target().unwrap();
        let head_commit = repo.find_commit(head_oid)?;
        let head_tree = head_commit.tree()?;

        snapbox::assert_eq(
            r#""bar" 778e23a1e80b1feb10e00b15b29a33315929c5b5
"foo.txt" 19102815663d23f8b75a47e7a01965dcdc96468c
"initial.txt" 63af22885f8665a312ba8b83db722134f1f8290d
"xyzzy.txt" 7c465afc533f95ff7d2c91e18921f94aac8292fc
"#,
            dump_tree_entries(&head_tree),
        );

        {
            let hydrated_tree = {
                let hydrated_tree_oid = rebuild_tree(&repo, Some(&head_tree), {
                    let mut result = HashMap::new();
                    result.insert(
                        std::path::PathBuf::from("foo-copy.txt"),
                        Some((
                            head_tree
                                .get_path(&std::path::PathBuf::from("foo.txt"))?
                                .id(),
                            0o100644,
                        )),
                    );
                    result.insert(std::path::PathBuf::from("foo.txt"), None);
                    result
                })?;
                repo.find_tree(hydrated_tree_oid)?
            };
            snapbox::assert_eq(
                r#""bar" 778e23a1e80b1feb10e00b15b29a33315929c5b5
"foo-copy.txt" 19102815663d23f8b75a47e7a01965dcdc96468c
"initial.txt" 63af22885f8665a312ba8b83db722134f1f8290d
"xyzzy.txt" 7c465afc533f95ff7d2c91e18921f94aac8292fc
"#,
                dump_tree_entries(&hydrated_tree),
            );
        }

        {
            let hydrated_tree = {
                let hydrated_tree_oid = rebuild_tree(&repo, Some(&head_tree), {
                    let mut result = HashMap::new();
                    result.insert(std::path::PathBuf::from("bar/bar.txt"), None);
                    result
                })?;
                repo.find_tree(hydrated_tree_oid)?
            };
            snapbox::assert_eq(
                r#""bar" 08ee88e1c53fbd01ab76f136a4f2c9d759b981d0
"foo.txt" 19102815663d23f8b75a47e7a01965dcdc96468c
"initial.txt" 63af22885f8665a312ba8b83db722134f1f8290d
"xyzzy.txt" 7c465afc533f95ff7d2c91e18921f94aac8292fc
"#,
                dump_tree_entries(&hydrated_tree),
            );
        }

        {
            let hydrated_tree = {
                let hydrated_tree_oid = rebuild_tree(&repo, Some(&head_tree), {
                    let mut result = HashMap::new();
                    result.insert(std::path::PathBuf::from("bar/bar.txt"), None);
                    result.insert(std::path::PathBuf::from("bar/baz.txt"), None);
                    result
                })?;
                repo.find_tree(hydrated_tree_oid)?
            };
            snapbox::assert_eq(
                r#""foo.txt" 19102815663d23f8b75a47e7a01965dcdc96468c
"initial.txt" 63af22885f8665a312ba8b83db722134f1f8290d
"xyzzy.txt" 7c465afc533f95ff7d2c91e18921f94aac8292fc
"#,
                dump_tree_entries(&hydrated_tree),
            );
        }

        {
            let dehydrated_tree_oid = filter_tree(
                &repo,
                &head_tree,
                &[
                    std::path::Path::new("bar/baz.txt"),
                    std::path::Path::new("foo.txt"),
                ],
            )?;
            let dehydrated_tree = repo.find_tree(dehydrated_tree_oid)?;
            snapbox::assert_eq(
                r#""bar" 08ee88e1c53fbd01ab76f136a4f2c9d759b981d0
"foo.txt" 19102815663d23f8b75a47e7a01965dcdc96468c
"#,
                dump_tree_entries(&dehydrated_tree),
            );
        }

        Ok(())
    }

    #[test]
    fn test_detect_path_only_changed_file_mode() -> eyre::Result<()> {
        let git = make_git()?;
        git.init_repo()?;

        git.run(&["update-index", "--chmod=+x", "initial.txt"])?;
        git.run(&["commit", "-m", "update file mode"])?;

        let repo = git.get_repo()?;
        let oid = repo.head()?.target().unwrap();
        let commit = repo.find_commit(oid)?;

        let lhs = commit.parent(0).unwrap();
        let lhs_tree = lhs.tree()?;
        let rhs_tree = commit.tree()?;
        let changed_paths =
            get_changed_paths_between_trees(&repo, Some(&lhs_tree), Some(&rhs_tree))?;
        let mut changed_paths = changed_paths
            .into_iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>();
        changed_paths.sort();

        snapbox::assert_eq(r#"["initial.txt"]"#, format!("{:?}", changed_paths));

        Ok(())
    }
}
