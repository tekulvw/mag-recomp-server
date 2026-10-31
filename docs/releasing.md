# Releases, container images and Helm chart

Release Please versions Medius, gameplay and the Helm chart independently. It
collects all pending component releases into **one release PR** and updates
`.release-please-manifest.json` once. A PR may contain one, two or three version
bumps; merging it publishes only those components. Independent versions do not
require separate PRs.

| Component | Path | Release tag | Version source | Artifact |
| --- | --- | --- | --- | --- |
| Medius, including embedded DME | `horizon/` | `medius-vX.Y.Z` | `horizon/version.txt` | Medius image |
| Gameplay | `gameplay/` | `gameplay-vX.Y.Z` | `gameplay/Cargo.toml` and `Cargo.lock` | Gameplay image |
| Helm chart | `charts/mag-recomp-server/` | `chart-vX.Y.Z` | `Chart.yaml` `version` | OCI Helm chart |

One combined PR avoids sibling release PRs conflicting on their central manifest.
When several components have pending releases, they share one review and merge;
they still retain separate versions, tags, release notes and publication gates.
Ordinary source changes can still require refreshing a stale release PR.

The Rust strategy keeps the gameplay crate and lockfile version in step with its
image version. The simple Medius strategy updates its version file and local
changelog. The Helm strategy updates chart `version` and its local changelog,
while preserving `appVersion` and explicit image tags. Operators can choose
Medius and gameplay versions separately in deployment values. Use a verified
compatible pair: a bridge protocol change can require changes and releases in
both server paths. Independent version numbers do not establish compatibility.

Runtime data revision is a separate operator-controlled value. Artifact
publication never embeds or increments the external game catalogs.

## Migration from the initial shared release

The historical `v0.1.0` release published both server images at `0.1.0`. That tag,
release and its image versions remain intact; the root `CHANGELOG.md` remains
historical. The obsolete root `version.txt` is replaced by Medius's version file.
The chart's existing `chart-v0.1.0` release also remains intact.

Both server paths inherit version `0.1.0` in the manifest. The configuration's
`bootstrap-sha` points to the merged initial chart release, after the shared
application release. Until component releases are discoverable for both server
paths, this bounds history so already-published features are not replayed as new
releases. Once all configured component releases are discoverable, Release
Please uses their release SHAs and ignores the bootstrap boundary. No alias tags
or rewritten historical releases are needed. The migration itself is a `chore`
and does not request another application or chart release.

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
   `fix: reject an invalid packet`. Changed file paths route qualifying commits:
   `horizon/` to Medius, `gameplay/` to gameplay, and the chart directory to the
   chart. Scopes explain intent but do not override path routing. Root-only docs
   and workflow edits do not create component releases.
2. Release Please creates or updates one combined release PR. A fix in one
   server path bumps only that server; a chart-only fix bumps only the chart.
   A qualifying commit touching several paths can contribute to several
   component releases. There is no linked-version plugin. Before `1.0.0`, fixes
   bump patch versions and breaking changes bump minor versions.
3. Review and merge that release PR. The next workflow run creates only its
   component releases, with separate `medius-v`, `gameplay-v` and `chart-v` tags.
   A manual workflow run on `main` can also reconcile a merged release PR.
4. If either application releases, the shared application check suite runs once
   against its tagged source. The two application tags refer to the same merge
   commit when both release from the combined PR. After checks pass,
   `horizon--release_created` enables only Medius image publication, while
   `gameplay--release_created` enables only gameplay image publication. Each
   publisher checks out its own component tag and uses that component's plain
   version output for image tags.
5. `charts/mag-recomp-server--release_created` independently enables tagged chart
   checks and OCI publication. Chart-only releases skip server compilation and
   image publication. The chart version does not advance either image tag.

Publishing stays in the Release Please workflow because a release made with
`GITHUB_TOKEN` does not start another release/tag workflow. A source push or
manual run that creates no release publishes no artifacts. Dispatching from a
branch other than `main` skips release creation. The reusable image publisher
requires read-only source access and package-write access; it receives no
broad repository-write permission.

`tests/release-routing.cjs` exercises the real Release Please library version
bundled by the pinned action against offline synthetic GitHub adapters. CI
checks historical bootstrap, independent path routing and version bumps, the
single combined PR and complete manifest, generated release tags, Cargo/Helm
updates, and each publication gate. It makes no GitHub writes or releases.

## Image names and tags

| Image | Contents and default entrypoint |
| --- | --- |
| `ghcr.io/tekulvw/mag-recomp-server-medius` | Horizon Medius server; also includes DME and MUIS binaries; starts `dotnet Server.Medius.dll` |
| `ghcr.io/tekulvw/mag-recomp-server-game` | Rust gameplay server and navigation tool; starts `mag-gameplay` |

The workflow derives the lower-case namespace from the repository owner, so forks
publish to their own owner namespace. Images currently target `linux/amd64`.
Each stable component release publishes its own full version, moving minor tag
and `latest` on its image name. For example, `medius-v0.1.1` updates only the
Medius image with `0.1.1`, `0.1` and `latest`; the gameplay image remains at its
separately released version. The historical `v0.1.0` published `0.1.0` on both names. Semver prereleases receive only their full prerelease
version tag; they do not advance minor or `latest` aliases. Use a full version or
image digest when a deployment needs a fixed image. Minor and `latest` tags move
on later stable releases of that image component.

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
release job's outputs and retries the failed jobs against the release tag. When both applications release, their
publish jobs are independent, so one can succeed while the other needs a retry.
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
