# Releases and container images

Release Please maintains one root release for both server components. It updates
`version.txt`, `CHANGELOG.md` and `.release-please-manifest.json` in a release PR.
The `simple` strategy deliberately leaves the gameplay crate's internal package
version independent of the shared distribution version. The empty initial
manifest means no release has shipped; `initial-version` sets the first proposed
release to `0.1.0`.

## Repository setup

Enable GitHub Actions and allow the pinned actions used by the workflows. In
**Settings > Actions > General > Workflow permissions**, enable **Allow GitHub
Actions to create and approve pull requests**. Organization policies must also
allow these operations. The workflows request permissions for individual jobs;
the repository does not need a globally writable default token.

The built-in `GITHUB_TOKEN` can create release PRs and releases and publish both
images. No registry secret is required. An optional `RELEASE_PLEASE_TOKEN` secret
can provide a GitHub App installation token or suitable personal access token
when the repository needs release PR events to trigger other workflows. GitHub
suppresses most events caused by `GITHUB_TOKEN`, including checks on a release PR
it creates. Without the optional token, maintainers can close and reopen that PR
to trigger the normal pull request checks. The release workflow always checks
the tagged source before publishing images.

Each GHCR package is connected to the source repository by its OCI source label.
GitHub creates new packages with private visibility even when the source repo is
public. After the first successful publication, set each package's visibility
to **Public** if anonymous image pulls are intended. If a package with either
name already exists, grant this repository Actions write access to it before
publishing. See GitHub's [container registry guide](https://docs.github.com/en/packages/working-with-a-github-packages-registry/working-with-the-container-registry)
and [package access settings](https://docs.github.com/en/packages/learn-github-packages/configuring-a-packages-access-control-and-visibility).

## Release flow

1. Merge Conventional Commits into `main`, such as `feat: add an objective` or
   `fix: reject an invalid packet`. Release Please creates or updates a release
   PR after a qualifying change. Before version `1.0.0`, breaking changes bump
   the minor version; fixes bump the patch version.
2. Review and merge that PR. The next workflow run creates a `vX.Y.Z` GitHub
   release. A manual run on `main` can also reconcile a merged release PR.
3. The `release_created` output enables the reusable public checks for that tag.
   On success, a matrix builds and pushes the Medius and gameplay images from
   the same tag using their separate `horizon/` and `gameplay/` contexts.

Publishing stays in the Release Please workflow because a release made with
`GITHUB_TOKEN` does not start another release/tag workflow. A normal source push
or manual workflow run that creates no release publishes no images. A workflow
dispatch from a branch other than `main` skips release creation.

## Image names and tags

| Image | Contents and default entrypoint |
| --- | --- |
| `ghcr.io/tekulvw/mag-recomp-server-medius` | Horizon Medius server; also includes DME and MUIS binaries; starts `dotnet Server.Medius.dll` |
| `ghcr.io/tekulvw/mag-recomp-server-game` | Rust gameplay server and navigation tool; starts `mag-gameplay` |

The workflow derives the lower-case namespace from the repository owner, so forks
publish to their own owner namespace. Images currently target `linux/amd64`.
Stable release `v0.1.0`, for example, publishes `0.1.0`, `0.1` and `latest` on
both names. Semver prereleases receive only their full prerelease version tag;
they do not advance minor or `latest` aliases. Use a full version or image digest
when a deployment needs a fixed image. Minor and `latest` tags move on later
stable releases.

```sh
docker pull ghcr.io/tekulvw/mag-recomp-server-medius:0.1.0
docker pull ghcr.io/tekulvw/mag-recomp-server-game:0.1.0
```

These commands work after that version has been released and its packages are
public or the operator has authenticated. Images contain no game files, account
database, credentials or usable production configuration. Supply permitted
runtime resources separately as described in [resource-boundary.md](resource-boundary.md)
and the [Horizon runtime guide](../horizon/README.md).

## Failed publication

The GitHub release is created before image checks and publication. If a check or
image job fails, correct transient runner/registry access problems and choose
**Re-run failed jobs** on that same run. This preserves the successful release
job's outputs and retries the failed jobs against the release tag. Both image
jobs are independent, so one can succeed while the other needs a retry.

A fresh manual run does not republish an existing release because Release Please
will not report it as newly created. Source defects require a fix and a new
release PR; preserve existing release tags. The workflow does not deploy a
service or start a server.

## Upstream references

- [Release Please action and token behavior](https://github.com/googleapis/release-please-action)
- [Manifest release configuration](https://github.com/googleapis/release-please/blob/v17.6.0/docs/manifest-releaser.md)
- [Docker semver metadata rules](https://github.com/docker/metadata-action#typesemver)
- [GitHub token permissions](https://docs.github.com/en/actions/tutorials/authenticate-with-github_token)
