# Maintaining Salat

How the maintainers verify and merge pull requests, what protects the
repository, and what to do when something goes wrong. Contributors need
only [`CONTRIBUTING.md`](../CONTRIBUTING.md); this guide is for the people
who merge.

## The model

- **GitHub is canonical**: issues, pull requests and Releases live at
  <https://github.com/linuxcoding-ADAM/Salat>.
- **GitLab CI/CD is the only automated CI** (`.gitlab-ci.yml`, project
  <https://gitlab.com/linuxcoding-ADAM/salat>). GitHub Actions is not used.
- **GitLab results do not appear on GitHub.** There are no GitHub status
  checks, so nothing on a pull request shows whether CI passed, and a
  contributor cannot start a pipeline. The maintainer runs it and reports
  the result by hand (below).
- **Only verified commits reach GitHub `main`, and they arrive unchanged.**
  A commit is verified when a GitLab pipeline has passed on that exact
  commit ID. Squash and rebase would change the ID, so they are never used.

## What protects the repository

GitHub repository rules, created on 2026-11-25. Neither has bypass actors,
so they apply to every account, including the repository admin.

| Ruleset | Matches | Blocks |
|---|---|---|
| `protect-main` | branch `main` | deleting it; non-fast-forward updates (force-pushes) |
| `protect-release-tags` | tags `v*` | deleting a tag; updating (moving) a tag; non-fast-forward updates |

They were tested on throwaway refs before being applied: a force-push and a
deletion were rejected, tag creation and an ordinary fast-forward push were
accepted. The rules require no pull request, no review and no status check.
An ordinary fast-forward push to `main` works, which is how the verified
flow below lands commits. New `v*` tags can still be created.

Merge methods: only **merge commits** are enabled on GitHub (squash and
rebase are disabled), because GitHub requires at least one. This is not an
invitation to use the button — see below.

**What the rules do not do.** People with the write role (the owner and
the collaborator `simoosh`) can still push straight to `main` and can still
click GitHub's merge button. Either puts a commit on `main` that GitLab
never verified. No rule prevents it; it is a working agreement, and the one
place where this process depends on discipline rather than tooling.

**Changing or removing a rule.** If a rule ever has to be lifted (say, to
remove a mistaken tag), an admin sets the ruleset's enforcement to
*Disabled* under *Settings → Rules → Rulesets*, does the operation, and sets
it back to *Active*. That is deliberate: there is no bypass to use by
accident. Do not leave a ruleset disabled.

## Verifying and merging a pull request

Throughout, `N` is the pull request number.

### 1. Triage

Read the diff. If the pull request touches any of these, read it **line by
line before mirroring anything**, because pushing a branch to GitLab runs
the pull request's own CI definition on a shared runner:

- `.gitlab-ci.yml`, `ci/`
- `install.sh`, `packaging/`
- `Cargo.toml`, `Cargo.lock`, `deny.toml`, any `build.rs`, `xtask/`

### 2. Mirror the exact commits to GitLab

```sh
N=<number>
SHA=$(gh pr view $N --json headRefOid -q .headRefOid)    # the commit under review
git fetch origin pull/$N/head                             # also works for fork pull requests
[ "$(git rev-parse FETCH_HEAD)" = "$SHA" ] || echo "STOP: the head moved"
git push gitlab "$SHA":refs/heads/pr/$N
```

If a check prints `STOP`, do not continue: the commit you are looking at is
not the one that was reviewed.

`pr/*` branches are not protected on GitLab (only `main` and `v*` are), so
their pipelines cannot read the protected release token. Do not add a
protection rule that matches `pr/*`.

### 3. Wait for the pipeline, and report it

Open the pipeline for `pr/N` on GitLab and wait for it to pass. The
installer jobs are manual on branches and are not needed for an ordinary
change. Post the result on the pull request, with the commit ID and the
link, using the checklist below. A pipeline that passes for `SHA` says
nothing about any other commit.

If it fails, link the failing job on the pull request and ask for a fix.
New commits mean a new `SHA`: start again from step 2.

One full pipeline is about 21 minutes of GitLab job time (measured on
2026-11-25), on a Free plan with a monthly compute limit. Do not mirror
pull requests that are not ready.

### 4. Integrate without changing any commit ID

Re-check that the pull request's head is still `SHA`, then update local
`main`:

```sh
[ "$(gh pr view $N --json headRefOid -q .headRefOid)" = "$SHA" ] || echo "STOP: the head moved"
git fetch origin
git switch main && git merge --ff-only origin/main
```

**If `main` has not moved** (`git merge-base --is-ancestor origin/main "$SHA"`
succeeds), fast-forward to the verified commits:

```sh
git merge --ff-only "$SHA"
git push origin main
```

**If `main` has moved**, either ask the contributor to update their branch
(a new `SHA`; start again), or create one merge commit yourself and verify
that commit too. The pull request's own commits keep their IDs:

```sh
git switch -c merge/pr-$N origin/main
git merge --no-ff -m "Merge pull request #$N: <title>" "$SHA"
M=$(git rev-parse HEAD)
git push gitlab "$M":refs/heads/pr/$N-merge
# wait for the pipeline to pass on $M, then:
git switch main && git merge --ff-only "$M"
git push origin main
```

`git push origin main` is a fast-forward, which the repository rules allow.
GitHub should mark the pull request as merged by itself, because its head
commit is now part of `main`. That has not been seen yet (the only pull
request so far, #1, was closed unmerged), so confirm it on the first real
one; if it does not happen, close the pull request with a link to the
commit.

### 5. Keep GitLab in step, then clean up

```sh
git push gitlab main
git push gitlab --delete pr/$N          # and pr/$N-merge if you made one
```

Pushing `main` to GitLab starts another full pipeline (about 21 more
minutes). The identical commit already passed on `pr/N`, so this run is a
confirmation. GitLab's `ci.skip` push option skips it; this project has not
used that option yet.

### Never

- GitHub's merge button, `gh pr merge`, or the web editor on `main`: each
  creates a commit on GitHub that GitLab never verified.
- Squash or rebase of a pull request: it changes the verified commit IDs.
- Pushing anything to GitHub `main` that has not passed a pipeline.

### Checklist to paste on the pull request

```
Maintainer checklist — PR #<N> @ <SHA>
- [ ] CI-affecting files reviewed line by line (or: the PR does not touch them)
- [ ] Mirrored to GitLab: pr/<N> at exactly <SHA>
- [ ] GitLab pipeline passed for <SHA>: <pipeline URL>
- [ ] Reviewed on GitHub
- [ ] Head is still <SHA> (checked just before integrating)
- [ ] Integrated by fast-forward, or by a verified merge commit <M> — not the merge button
- [ ] GitLab main in step; pr/<N> deleted
```

## When something goes wrong

| Situation | What to do |
|---|---|
| The contributor pushed after you mirrored | The head no longer matches `SHA`. Discard the old run and start again from step 2 with the new head. |
| `main` moved while a pipeline was running | The verified commits are no longer a fast-forward. Ask for an update, or use the verified merge commit path in step 4. |
| An unverified commit reached `main` (the merge button, or a direct push) | Force-pushes are blocked, so fix forward. Push it to GitLab (`git fetch origin && git push gitlab origin/main:main`) and let the pipeline judge it. If it is bad, add a revert commit through the normal flow. Do not lift `protect-main` to rewrite history. |
| A push to `main` is rejected with `GH013` | A repository rule blocked it: a force-push or deletion. Pull and fast-forward instead. |
| The pull request is not marked merged after `main` advanced | Close it with a link to the commit that contains its changes. |

## Releases

Releases follow the procedure in
[`docs/ARCHITECTURE.md`](ARCHITECTURE.md) §22.3, summarised in
[`CONTRIBUTING.md`](../CONTRIBUTING.md#releases). A `v*` tag, once pushed
to GitHub, cannot be moved or deleted while `protect-release-tags` is
active, so check the tag before pushing it.
