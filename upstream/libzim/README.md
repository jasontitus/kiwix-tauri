# Submitting the libzim patches

Each directory below holds one pull request for [openzim/libzim](https://github.com/openzim/libzim):

- the `git format-patch` output (`000N-*.patch`);
- `PR.md`, the pull request description. Its first line is the PR title and the rest is the body.

| Order | Directory | Commits |
|---|---|---|
| 1 | `1-lazy-fulltext-index/` | 1 |
| 2 | `2-lp32-oversized-raw-blob/` | 1 |
| 3 | `3-cluster-memory-accounting/` | 2 (keep both, in order) |

The patches are based on upstream `main` at `ad888b4`. Each applies cleanly on its own. They also apply together in the order above, and the full test suite passes with all four commits applied.

## Order

1. **`1-lazy-fulltext-index`** first: a one-line fix with a test.
2. **`2-lp32-oversized-raw-blob`**: independent of the others.
3. **`3-cluster-memory-accounting`**: its two commits must stay in one PR and in this order. The first adds the lock and the second fixes the zstd size. Do not split them, and do not submit the second one alone.

The PRs do not depend on each other. Numbers 2 and 3 both touch `src/cluster.cpp` and `test/cluster.cpp`, in different places. If one of them is merged first, the other may need a trivial rebase.

PRs 1 and 3 reference upstream issue #1127 with "Refs", because that issue lists many unrelated items and should not be closed by these PRs.

## Steps (for each PR)

```bash
# Once: fork openzim/libzim on GitHub, then
git clone https://github.com/<you>/libzim.git
cd libzim
git remote add upstream https://github.com/openzim/libzim.git

# For each PR directory <dir> (e.g. 1-lazy-fulltext-index) and branch name <branch>
git fetch upstream
git switch -c <branch> upstream/main
git am /path/to/upstream/libzim/<dir>/*.patch      # use `git am -3` if main has moved
```

The patches are authored as `Claude <noreply@anthropic.com>`. To submit them under your own name, rewrite the author before pushing:

```bash
git rebase --exec 'git commit --amend --no-edit --reset-author' upstream/main
```

Build and run the tests before pushing:

```bash
meson setup build -Dtest_data_dir=<dir>   # or: meson setup build && ninja -C build download_test_data
ninja -C build
meson test -C build
```

Each `PR.md` lists the specific test commands, including the 32-bit build for PR 2 and the ThreadSanitizer build for PR 3.

Notes on the test environment:

- Some cluster tests write a 4 GiB file to `$TMPDIR`. On a disk with little free space, set `TMPDIR` to a larger filesystem (for example a tmpfs), or set `SKIP_BIG_MEMORY_TEST=1`.
- Some tests need up to 16 GB of memory (see the libzim README).

Push and open the PR:

```bash
git push -u origin <branch>
gh pr create --repo openzim/libzim --base main --head <you>:<branch> \
  --title "$(head -1 /path/to/upstream/libzim/<dir>/PR.md | sed 's/^# //')" \
  --body "$(tail -n +3 /path/to/upstream/libzim/<dir>/PR.md)"
```

You can also open the PR from the GitHub web page and paste `PR.md`: its first line as the title, the rest as the body.

Suggested branch names:

- `fix-lazy-xapian-db`
- `fix-lp32-oversized-raw-blob`
- `fix-cluster-memory-accounting`
