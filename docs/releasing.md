# Releases, container images and Helm chart

Release Please maintains a root release for both server components. It updates
`version.txt`, `CHANGELOG.md` and `.release-please-manifest.json` in a release PR.
The `simple` strategy deliberately leaves the gameplay crate's internal package
version independent of the shared distribution version. The Helm chart has its
own release path, `charts/mag-recomp-server`, and a separate release PR. The empty
initial manifest means neither component has shipped; each starts at `0.1.0`
and advances independently thereafter.

| Component | Release tag | Version source | Published artifacts |
| --- | --- | --- | --- |
| Server application | `vX.Y.Z` | Root `version.txt` | Medius and gameplay container images |
| Helm chart | `chart-vX.Y.Z` | `charts/mag-recomp-server/Chart.yaml` `version` | OCI Helm chart in GHCR |

The chart uses Release Please's `helm` strategy, which updates `version` and its
chart-local changelog. It leaves `appVersion` and image tags unchanged. Those
fields describe the application images selected by the chart and are updated
explicitly when changing the supported application release. Runtime data revision
is a third, operator-controlled value: chart or image publication never embeds or
increments an operator's external game data.

## Repository setup

Enable GitHub Actions and allow the pinned actions used by the workflows. In
**Settings > Actions > General > Workflow permissions**, enable **Allow GitHub
Actions to create and approve pull requests**. Organization policies must also
allow these operations. The workflows request permissions for individual jobs;
the repository does not need a globally writable default token.

The built-in `GITHUB_TOKEN` can create release PRs and releases and publish both
images and the OCI chart. No registry secret is required. An optional
`RELEASE_PLEASE_TOKEN` secret can provide a GitHub App installation token or
suitable personal access token when the repository needs release PR events to
trigger other workflows. GitHub suppresses most events caused by `GITHUB_TOKEN`,
including checks on a release PR it creates. Without the optional token,
maintainers can close and reopen that PR to trigger the normal pull request
checks. The release workflow always checks the tagged source before publication.

Each container image is connected to the source repository by its OCI source
label. GitHub creates new packages with private visibility even when the source
repo is public. After the first successful publication, set each package's
visibility to **Public** if anonymous pulls are intended. If a package with the
same name already exists, grant this repository Actions write access to it
before publishing. Apply the same visibility and repository Actions access setup
to `ghcr.io/OWNER/charts/mag-recomp-server`. See GitHub's
[container registry guide](https://docs.github.com/en/packages/working-with-a-github-packages-registry/working-with-the-container-registry)
and [package access settings](https://docs.github.com/en/packages/learn-github-packages/configuring-a-packages-access-control-and-visibility).

## Release flow

1. Merge Conventional Commits into `main`, such as `feat: add an objective` or
   `fix: reject an invalid packet`. Release Please creates or updates a release
   PR after a qualifying change. Before version `1.0.0`, breaking changes bump
   the minor version; fixes bump the patch version.
2. Changes confined to `charts/mag-recomp-server/` are excluded from the root
   application release. A qualifying chart change, such as `fix(chart): correct
   a service port`, creates or updates the chart release PR. Commit scope helps
   explain intent; changed file paths determine routing. Keep chart-only changes
   in their own commits: a qualifying commit touching chart and application files
   can contribute to both releases. No linked-version plugin couples them.
3. Review and merge the relevant release PR. The next workflow run creates a
   `vX.Y.Z` application release or `chart-vX.Y.Z` chart release. A manual run on
   `main` can also reconcile a merged release PR.
4. The root `release_created` output enables public checks for the application
   tag. On success, a matrix builds and pushes the Medius and gameplay images
   from that tag using separate `horizon/` and `gameplay/` contexts.
5. The `charts/mag-recomp-server--release_created` output independently enables
   chart checks for the chart tag: `helm lint --strict`, `helm template` with
   synthetic CI values, and `helm package`. After those pass, the workflow logs
   in with `GITHUB_TOKEN` and uses `helm push` to publish the OCI chart. A
   chart-only release skips server compilation and image publication.

Publishing stays in the Release Please workflow because a release made with
`GITHUB_TOKEN` does not start another release/tag workflow. A normal source push
or manual workflow run that creates no release publishes no artifacts. A workflow
dispatch from a branch other than `main` skips release creation.

## Image names and tags

| Image | Contents and default entrypoint |
| --- | --- |
| `ghcr.io/tekulvw/mag-recomp-server-medius` | Horizon Medius server; also includes DME and MUIS binaries; starts `dotnet Server.Medius.dll` |
| `ghcr.io/tekulvw/mag-recomp-server-game` | Rust gameplay server and navigation tool; starts `mag-gameplay` |

The workflow derives the lower-case namespace from the repository owner, so forks
publish to their own owner namespace. Images currently target `linux/amd64`.
Stable application release `v0.1.0`, for example, publishes `0.1.0`, `0.1` and
`latest` on both names. Semver prereleases receive only their full prerelease
version tag; they do not advance minor or `latest` aliases. Use a full version or
image digest when a deployment needs a fixed image. Minor and `latest` tags move
on later stable application releases.

```sh
docker pull ghcr.io/tekulvw/mag-recomp-server-medius:0.1.0
docker pull ghcr.io/tekulvw/mag-recomp-server-game:0.1.0
```

These commands work after that version has been released and its packages are
public or the operator has authenticated. Images contain no game files, account
database, credentials or usable production configuration. Supply permitted
runtime resources separately as described in [resource-boundary.md](resource-boundary.md)
and the [Horizon runtime guide](../horizon/README.md).

## OCI Helm chart

The chart is published to `ghcr.io/tekulvw/charts/mag-recomp-server` with the
chart's full SemVer version. Helm derives the OCI artifact name and version from
`Chart.yaml`; the workflow verifies that the package filename matches the chart
release output. There is no moving `latest` chart tag. Forks use their own
lower-case owner namespace.

After a chart release has been published and its GHCR package made public:

```sh
helm show values oci://ghcr.io/tekulvw/charts/mag-recomp-server --version 0.1.0
helm pull oci://ghcr.io/tekulvw/charts/mag-recomp-server --version 0.1.0
helm upgrade --install mag oci://ghcr.io/tekulvw/charts/mag-recomp-server \
  --version 0.1.0 --namespace mag --create-namespace --values operator-values.yaml
```

Prepare `operator-values.yaml` and required external resources using the
[chart deployment guide](../charts/mag-recomp-server/README.md) before installing.
The synthetic `ci-values.yaml` is for render validation. It supplies documentation
addresses and cannot configure a usable game service. For a private package,
authenticate with `helm registry login ghcr.io` before pulling or installing.
Changing chart version does not automatically advance its pinned application
image tags or external data revision.

## Failed publication

The GitHub release is created before artifact checks and publication. If a check
or publication job fails, correct transient runner/registry access problems and
choose **Re-run failed jobs** on that same run. This preserves the successful
release job's outputs and retries the failed jobs against the release tag. Both
image jobs are independent, so one can succeed while the other needs a retry.
Chart publication likewise uses its own tag and check dependency.

A fresh manual run does not republish an existing release because Release Please
will not report it as newly created. Source defects require a fix and a new
release PR for the affected component; preserve existing release tags. The
workflow does not deploy a service or start a server.

## Upstream references

- [Release Please action and token behavior](https://github.com/googleapis/release-please-action)
- [Manifest release configuration](https://github.com/googleapis/release-please/blob/v17.6.0/docs/manifest-releaser.md)
- [Path-based release exclusions](https://github.com/googleapis/release-please/blob/v17.6.0/src/util/commit-exclude.ts)
- [Helm chart version updater](https://github.com/googleapis/release-please/blob/v17.6.0/src/updaters/helm/chart-yaml.ts)
- [Helm OCI registries and package naming](https://helm.sh/docs/topics/registries/)
- [Docker semver metadata rules](https://github.com/docker/metadata-action#typesemver)
- [GitHub token permissions](https://docs.github.com/en/actions/tutorials/authenticate-with-github_token)
