# Rolling main container channel

**Goal:** Publish `main` tags for gameplay and Horizon/Medius after checked pushes to main, allowing manual prerelease testing without cutting a release. The user explicitly removed chart publication from this task.

**Approach:** Add a dedicated workflow rather than changing the stable release publisher. Reuse the public checks, build both images with full source-SHA tags, then promote their `main` aliases only after both builds succeed and the checked commit is still current main. Serialize runs and reject off-main manual dispatches. No Git tags, GitHub releases, chart publication or live deployments.

**Files:** `.github/workflows/main-images.yml`, `tests/test_main_images.py`, `docs/releasing.md`, `README.md`.

**Checks and steps:**
- [x] Add a failing workflow/promotion check covering main-only events, both builds/check dependencies, same source SHA, stale-run rejection, image-only promotion and isolated release tags.
- [x] Implement the dedicated workflow with pinned existing checkout, Buildx, login and build actions, per-job permissions and source revision labels.
- [x] Document image names, authenticated pulls, Always pull policy, clearing digest pins and explicit deployment rollout for retesting a moving tag.
- [x] Run the Python suite, Release Please routing fixture, workflow lint and public-tree guard; review the focused diff and commit locally.

**Review focus:** Both images must build before either main alias moves. Old reruns cannot overwrite a newer main commit. Pull requests and off-main dispatches cannot publish. Stable release tags/publisher and chart defaults remain unchanged. Moving tags do not restart Kubernetes Pods automatically.

## Verification result

The baseline passed126 Python tests. The new checks first failed for the missing
workflow and missing overlay. Final Python verification passes133 tests, including
actual promotion shell execution with synthetic registry/API boundaries and real
Helm rendering with previously pinned digests. The unchanged Release Please
routing fixture passes. The public-tree guard and diff whitespace check pass.

Read-only source review identified pending-run eviction by old retries. The fix
uses branch-specific concurrency groups and GitHub's documented `queue: max`;
a regression contract failed before the fix and passes afterward. Actionlint
1.7.12 validates all remaining workflow syntax/expressions; it does not yet
recognize `concurrency.queue`, so that one diagnostic was filtered after checking
the current official GitHub workflow syntax reference.

No chart source or stable publisher was changed. No image was published locally,
no deployment was changed, and no release was created. The main channel starts
when this workflow is integrated into main and its checks/builds succeed.
