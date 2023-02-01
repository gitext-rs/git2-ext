# Change Log
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](http://keepachangelog.com/)
and this project adheres to [Semantic Versioning](http://semver.org/).

<!-- next-header -->
## [Unreleased] - ReleaseDate

## [0.5.0] - 2023-02-01

### Fixes

- Upgrade git2

## [0.4.3] - 2023-01-25

### Fixes

- Split out `ops::signature` into `ops::commit_signature` and `ops::author_signature`

## [0.4.2] - 2023-01-25

### Features

- `git2_ext::ops::signature` replacement for `Repository::signature` that respects
 `GIT_COMMITTER_NAME` and `GIT_COMMITTER_EMAIL`

### Fixes

- For signing and cherry-picking, respect `GIT_COMMITTER_NAME` and `GIT_COMMITTER_EMAIL`

## [0.4.1] - 2023-01-25

### Fixes

- Pass correct signature for signing when no key is specified

## [0.4.0] - 2023-01-12

### Breaking changes

- `cherry_pick` gained a `sign` argument.  Pass `None` to get the old behavior

### Features

- Allow signing `cherry_pick` operations

## [0.3.0] - 2023-01-05

### Breaking changes

- `Sign::sign` now returns `Result<String, git2::Error>`

### Compatibility

MSRV bumped to 1.64.0

### Features

- `GpgSign` and `SshSign` implementations of `Sign`
- `UserSign` implementation of `Sign` that does all of the right things based on the config

## [0.2.0] - 2023-01-05

### Breaking changes

- `reword` and `squash` gained `sign` argument.  Pass `None` to get the old behavior

### Features

- New `commit` function that handles signing for you
- Signing support for `reword` and `squash`

## [0.1.0] - 2022-12-02

### Features

- Commit filtering

## [0.0.7] - 2022-10-03

### Breaking Changes

- Upgraded `git2`

## [0.0.6] - 2022-06-20

### Features

- Reword support

## [0.0.5] - 2022-03-14

### Breaking Change

- Made clear which hooks are never-fail

### Features

- Added `hooks::ReferenceTransaction` to clarify how API should be used

### Fixes

- `ops::cherry_pick` correctly picks parent commit
- Only log hook exit code on failure

## [0.0.4] - 2022-03-12

### Breaking Change

- Renamed from git2ext to git2-ext

## [0.0.3] - 2022-03-11

### Features

- cherry-pick and squash support
- Wrappers around git hooks

<!-- next-url -->
[Unreleased]: https://github.com/gitext-rs/git2-ext/compare/v0.5.0...HEAD
[0.5.0]: https://github.com/gitext-rs/git2-ext/compare/v0.4.3...v0.5.0
[0.4.3]: https://github.com/gitext-rs/git2-ext/compare/v0.4.2...v0.4.3
[0.4.2]: https://github.com/gitext-rs/git2-ext/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/gitext-rs/git2-ext/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/gitext-rs/git2-ext/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/gitext-rs/git2-ext/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/gitext-rs/git2-ext/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/gitext-rs/git2-ext/compare/v0.0.7...v0.1.0
[0.0.7]: https://github.com/gitext-rs/git2-ext/compare/v0.0.6...v0.0.7
[0.0.6]: https://github.com/gitext-rs/git2-ext/compare/v0.0.5...v0.0.6
[0.0.5]: https://github.com/gitext-rs/git2-ext/compare/v0.0.4...v0.0.5
[0.0.4]: https://github.com/gitext-rs/git2-ext/compare/v0.0.3...v0.0.4
[0.0.3]: https://github.com/gitext-rs/git2-ext/compare/15449592300986753c174f63d412b212ad919285...v0.0.3
