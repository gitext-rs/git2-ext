#[test]
fn cherry_pick_clean() {
    let temp = assert_fs::TempDir::new().unwrap();
    let plan = git_fixture::Dag::load(std::path::Path::new("tests/fixtures/branches.yml")).unwrap();
    plan.run(temp.path()).unwrap();

    let repo = git2::Repository::discover(temp.path()).unwrap();

    {
        assert!(!git2ext::ops::is_dirty(&repo));
        let expected_head_id = git2ext::ops::head_id(&repo).unwrap();

        let base = repo
            .find_branch("off_master", git2::BranchType::Local)
            .unwrap();
        let base_id = base.get().target().unwrap();
        let source = repo
            .find_branch("feature1", git2::BranchType::Local)
            .unwrap();
        let source_id = source.get().target().unwrap();

        let dest_id = git2ext::ops::cherry_pick(&repo, base_id, source_id).unwrap();

        let source_commit = repo.find_commit(source_id).unwrap();
        let dest_commit = repo.find_commit(dest_id).unwrap();
        let actual_head_id = git2ext::ops::head_id(&repo).unwrap();

        assert_ne!(dest_id, source_id);
        assert_eq!(dest_commit.message(), source_commit.message());
        assert_eq!(expected_head_id, actual_head_id);
        assert!(!git2ext::ops::is_dirty(&repo));
    }

    temp.close().unwrap();
}

#[test]
fn cherry_pick_conflict() {
    let temp = assert_fs::TempDir::new().unwrap();
    let plan = git_fixture::Dag::load(std::path::Path::new("tests/fixtures/conflict.yml")).unwrap();
    plan.run(temp.path()).unwrap();

    let repo = git2::Repository::discover(temp.path()).unwrap();

    {
        assert!(!git2ext::ops::is_dirty(&repo));

        let base = repo
            .find_branch("feature1", git2::BranchType::Local)
            .unwrap();
        let base_id = base.get().target().unwrap();
        let source = repo.find_branch("master", git2::BranchType::Local).unwrap();
        let source_id = source.get().target().unwrap();

        let dest_id = git2ext::ops::cherry_pick(&repo, base_id, source_id);

        println!("{:#?}", dest_id);
        assert!(dest_id.is_err());
        assert!(!git2ext::ops::is_dirty(&repo));
    }

    temp.close().unwrap();
}

#[test]
fn squash_clean() {
    let temp = assert_fs::TempDir::new().unwrap();
    let plan = git_fixture::Dag::load(std::path::Path::new("tests/fixtures/branches.yml")).unwrap();
    plan.run(temp.path()).unwrap();

    let repo = git2::Repository::discover(temp.path()).unwrap();

    {
        assert!(!git2ext::ops::is_dirty(&repo));

        let base = repo.find_branch("master", git2::BranchType::Local).unwrap();
        let base_id = base.get().target().unwrap();
        let source = repo
            .find_branch("feature1", git2::BranchType::Local)
            .unwrap();
        let source_id = source.get().target().unwrap();

        let dest_id = git2ext::ops::squash(&repo, source_id, base_id).unwrap();

        println!("{:#?}", dest_id);
        assert!(!git2ext::ops::is_dirty(&repo));
    }

    temp.close().unwrap();
}
