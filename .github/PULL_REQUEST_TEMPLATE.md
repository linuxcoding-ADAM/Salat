## What and why

<!-- What does this change, and why? Link the issue it fixes, if there is one. -->

## Checks I ran locally

<!-- Salat's CI runs on GitLab, not on GitHub, so nothing is checked automatically here. -->

- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] `cargo test --all-features` (as a normal user, not root)
- [ ] If screens changed: I reviewed the snapshot diff and every changed line is intended
- [ ] If behaviour changed: I updated the matching document and added a line under `[Unreleased]` in `CHANGELOG.md`

## How this pull request is verified

No automated checks run on GitHub, and GitLab results do not appear on this
page. The maintainer copies the exact commits of this pull request to GitLab,
runs the project's CI there, and posts the result as a comment. The result
applies to one exact commit: if you push new commits, they are verified again.

Verified commits are added to `main` with their commit IDs unchanged. GitHub's
merge button is not used, and squash and rebase merging are disabled. See
[CONTRIBUTING.md](https://github.com/linuxcoding-ADAM/Salat/blob/main/CONTRIBUTING.md#sending-a-pull-request).
