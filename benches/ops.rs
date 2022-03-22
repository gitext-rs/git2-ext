use std::collections::HashSet;

fn get_repo() -> git2::Repository {
    let repo_dir =
        std::env::var("PATH_TO_REPO").expect("`PATH_TO_REPO` environment variable not set");
    git2::Repository::discover(&std::path::PathBuf::from(repo_dir)).unwrap()
}

fn bench_get_changed_paths_between_trees(c: &mut criterion::Criterion) {
    c.bench_function("get_changed_paths_between_trees", |b| {
        let repo = get_repo();
        let oid = repo.head().unwrap().target().unwrap();
        let commit = repo.find_commit(oid).unwrap();
        let parent = commit.parent(0).unwrap();
        let parent_tree = parent.tree().unwrap();
        let commit_tree = commit.tree().unwrap();

        b.iter(|| -> HashSet<std::path::PathBuf> {
            git2_ext::tree::get_changed_paths_between_trees(
                &repo,
                Some(&parent_tree),
                Some(&commit_tree),
            )
            .unwrap()
        });
    });
}

criterion::criterion_group!(benches, bench_get_changed_paths_between_trees,);
criterion::criterion_main!(benches);
