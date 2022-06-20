# Change Log
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](http://keepachangelog.com/)
and this project adheres to [Semantic Versioning](http://semver.org/).

<!-- next-header -->
## [Unreleased] - ReleaseDate

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
[Unreleased]: https://github.com/gitext-rs/git2-ext/compare/v0.0.5...HEAD
[0.0.5]: https://github.com/gitext-rs/git2-ext/compare/v0.0.4...v0.0.5
[0.0.4]: https://github.com/gitext-rs/git2-ext/compare/v0.0.3...v0.0.4
[0.0.3]: https://github.com/gitext-rs/git2-ext/compare/15449592300986753c174f63d412b212ad919285...v0.0.3
