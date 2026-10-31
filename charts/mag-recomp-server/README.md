# MAG server Helm chart

This chart deploys the source server with operator-supplied private runtime catalogs and stable keys. Kubernetes 1.27 or newer, an amd64 worker, application images, a mixed TCP/UDP LoadBalancer controller, and private runtime inputs are prerequisites. The chart contains no game data, accounts, credentials, certificates, or private addresses. It does not run a game client or deploy anything during packaging.

Chart `version` and application image tags have separate release lifecycles. Version 0.1.0 defaults to the independently selected `ghcr.io/tekulvw/mag-recomp-server-medius:0.1.0` and `ghcr.io/tekulvw/mag-recomp-server-game:0.1.0`. Set `images.medius.tag` and `images.gameplay.tag` explicitly when upgrading applications; a chart release does not change either tag automatically. The current source supports a captured development authentication profile. This chart explicitly enables that profile for Medius and MAPS; it is a development server deployment, and does not implement a production identity provider.

## Runtime layout

| Component | Placement | Network | Persistent state |
| --- | --- | --- | --- |
| Gameplay | One Pod | Private ClusterIP TCP 31000 | None; read-only catalogs |
| Main Medius + local DME | One container in the Medius Pod | TCP 10475 auth, 10478 lobby, 10477 proxy, external 10473 to container 10073; UDP 10474 | Account PVC read/write |
| MAPS | Second container in the same Medius Pod, same image | TCP `network.mapsPort` (default 10471) to auth 10471, 10472 lobby, 10476 proxy | Same account PVC read-only |
| Optional DNS | Separate Pod, disabled by default | TCP/UDP 53 to container port 1053 | None |

Medius and MAPS have separate `/config/main` and `/config/maps` directories in a writable `emptyDir`, with separate listener ports. The main role enables MAS, account login, character data and local DME. MAPS enables the plugin adapter and reads the same character store. Both load the same stable operator-supplied RSA keys. The source server's `MAG_ADVERTISE_IPV4`, `MAG_AUTH_DNS`, `MAG_AUTH_PORT`, `MAG_LOBBY_PORT` and `MAG_GAME_TCP_PORT` environment settings are provided by the chart. DME binds internally on TCP 10073 and UDP 10474; the Service maps the external TCP port. The advertised address must be reachable by the client and match the LoadBalancer address or the address actually routed to it.

The supported deployment is exactly one gameplay Pod and one Medius Pod, using `Recreate` updates. `replicaCount` values other than one are rejected. Horizontal scaling and rolling overlap would need additional coordination for session state, listeners and the account writer.

## Supply private inputs

Create an existing Kubernetes Secret in the release namespace using your private secret manager or an operator-only file. Set `runtime.existingSecret` to its name. Its required keys are `scert-mas-certificate.bin`, `scert-mls-certificate.bin`, and `medius-keys.json`. The MAS binary SCERT template must be at least 468 bytes, with its 64-byte RSA modulus field starting at offset 0x194; the MLS template must be nonempty. Both SCERT certificate templates must be compatible with the selected client and stable RSA keys; supply them privately from your existing runtime setup. The `medius-keys.json` key must hold a JSON object containing exactly `MPSKey` and `DefaultKey`; each has `n`, `e`, and `d` positive decimal strings matching the source `RsaKeyPairConverter`. Use your existing compatible stable key material. Do not put its content in Helm values, Git, chart artifacts, command-line literals, or a runtime-data bundle. The chart neither generates keys nor relies on ephemeral per-process development keys.

A Python init container merges these keys into public role settings, writes mode-0600 `medius.json` files in the writable configuration volume, and optionally copies the Secret's `db.config.json` into both role directories. The applications receive `HORIZON_SCERT_MAS_CERTIFICATE_PATH=/runtime-secrets/scert-mas-certificate.bin` and `HORIZON_SCERT_MLS_CERTIFICATE_PATH=/runtime-secrets/scert-mls-certificate.bin`. The complete Secret is mounted read-only at `/runtime-secrets` in both applications, so any separately supplied certificate files remain external. This chart does not create a TLS/SVO listener; DNS changes do not supply HTTPS certificates. When using an external database configuration, ensure its persistence and access policy separately. `simulated.db` is transient configuration-volume state; the MAG character/account store is the persistent `/accounts/characters.json` file.

By default, `accounts.existingClaim` is empty and the chart creates a retained `ReadWriteOnce` account PVC. Set `accounts.storageClass` to an appropriate block CSI class for your cluster; leaving it empty selects the cluster default. A private homelab using Synology block CSI can use its local `synostorage` class in private values. This chart does not install a storage driver or use host paths. Alternatively set `accounts.existingClaim` to an existing claim. Provision or migrate existing account files privately, retain their ownership, and allow UID/GID 10001 to read and write the main account store. MAPS mounts that claim read-only within the same Pod, avoiding an additional RWO attachment. Back up the account PVC separately; `helm.sh/resource-policy: keep` retains a chart-created claim on uninstall but is not a backup.

For catalogs, the default `data.mode: existingClaim` references the placeholder claim `mag-runtime-data`. This is a reference to operator-provisioned storage, not a fake chart-created data claim. Both application Pods mount it read-only at `/data`. Use RWX storage for Pods on different nodes, or enforce co-location with private scheduling values if the storage and driver support that access pattern. An RWO claim alone does not make independent Pods portable across nodes.

The recommended mode for RustFS or S3 is `data.mode: s3`. Each Pod downloads and verifies the same immutable bundle into its own `emptyDir`; accounts remain on their separate PVC. For example, a private values file can contain:

```yaml
network:
  advertisedIPv4: 203.0.113.10 # Documentation address: replace privately.
  authDns: auth.example.invalid
runtime:
  existingSecret: mag-runtime-keys
accounts:
  existingClaim: mag-accounts
data:
  mode: s3
  s3:
    endpoint: https://objects.example.invalid
    region: us-east-1
    bucket: private-runtime
    key: bundles/catalogs-v1.tar.gz
    versionId: "" # Set if your object store provides immutable version IDs.
    sha256: REPLACE_WITH_THE_64_CHARACTER_ARCHIVE_SHA256
    existingSecret: mag-object-store
    forcePathStyle: true
```

The object-store Secret uses `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY`, with optional `AWS_SESSION_TOKEN`. Credentials are visible only to the AWS CLI download init container through `envFrom`. Give them read access to the selected object, use HTTPS with a trusted certificate, and provide a custom trusted-CA AWS CLI image if your endpoint requires a private CA. No credentials are accepted as chart values. `endpoint` may be empty for AWS S3. The init container writes an AWS configuration file to a private writable scratch volume and selects path-style requests for RustFS when `forcePathStyle` is true. Object keys, region, endpoint and version are passed through environment variables rather than shell interpolation.

The verification init container runs the bundled public `data_bundle.py` with `unpack --archive /download/bundle.tar.gz --sha256 ... --output-dir /catalogs`. It verifies the archive checksum and per-file manifest, rejects unsafe members, then makes extracted files read-only. SHA-256 is mandatory even when a version ID is supplied. Use a fresh object key and checksum for every data update; do not overwrite a pinned release. See [private bundle preparation](../../docs/data-bundles.md) for the explicit input selection and packaging commands. The default verifier limits are 512 MiB per file, 2 GiB total/archive, and 4096 files; its 1 GiB memory limit is configurable through `resources.init` and must accommodate parsed JSON, not just compressed archive size. `data.emptyDirSizeLimit` bounds each archive and extracted-data volume separately; account for both when sizing node ephemeral storage. A completed data init container is not a live synchronizer. Failed verification prevents the application from starting.

The `data.files` table selects private paths relative to `/data`: required gameplay inputs are `items`, `spawns`, `wireProfiles`, and `skills`; Medius additionally consumes `loadoutDefaults`, `developmentProgression`, `trainingCompletion`, and `progression`. Retail application ID 20231 also selects `retailSkills` and `retailArmory`; these environment variables are omitted for prototype 20230 because the source rejects retail catalogs for that profile. All supplied catalogs must match the selected application and map. Additional gameplay catalog or collision options can be supplied as the string array `data.gameplayExtraArgs`, using `/data/...` paths. The required gameplay bind, event log and baseline catalog arguments are supplied by the chart; do not override them through extra arguments.

Catalog mounts are read-only in application containers. Logs and temporary files use writable `emptyDir` volumes. Gameplay creates a unique event-log directory on each container start because the server rejects reuse of an existing event-log path. The applications run as UID/GID 10001, drop Linux capabilities, use the runtime-default seccomp profile, and do not receive a service-account token. Application root filesystems remain writable for compatibility with Horizon's startup writes. The optional DNS image uses a read-only root filesystem and adds only the `NET_BIND_SERVICE` capability to its otherwise empty capability set, because the upstream image marks its binary with that file capability and cannot execute with it removed from the bounding set. It still runs without privilege escalation on port 1053.

## Optional forwarding DNS

Set `dns.enabled: true`, `dns.answerIPv4` to the reachable server address, and `dns.hostnames` to the exact original game hostnames observed for the client build. Retained retail client evidence identifies `mag.ps3.online.scea.com` as the original MAPS hostname; the prototype uses numeric addresses and cannot be redirected by DNS alone. The default hostname list stays empty so enabling DNS always requires an explicit choice. The observed original retail settings use TCP 10073 and UDP 10072 for that bootstrap endpoint. The example below overrides only the retail MAPS hostname and maps its TCP bootstrap port to the chart MAPS listener. Supply a separately verified compatible retail runtime profile and matching private catalogs; this snippet does not select a protocol version:

```yaml
network:
  mapsPort: 10073
dns:
  enabled: true
  answerIPv4: 203.0.113.10
  hostnames:
    - mag.ps3.online.scea.com
  upstreams:
    - /etc/resolv.conf
  service:
    type: LoadBalancer
    loadBalancerSourceRanges:
      - 203.0.113.0/24 # Replace privately with your LAN or VPN client CIDRs.
```

The pinned CoreDNS 1.12.4 Deployment listens without privileges on port 1053; its Service exposes both UDP and TCP 53. Exact host entries return the configured IPv4. Unrelated questions fall through to the configured upstream resolver, with no reverse records generated. This is a forwarding resolver with selected host overrides; it does not configure delegated authoritative zones or change public DNS. The CoreDNS header plugin clears the AA bit and sets RA on responses, including local host overrides. The cache plugin is omitted because its cached response path can bypass those header changes in the selected release. A/AAAA/PTR behavior follows the [CoreDNS hosts plugin](https://coredns.io/plugins/hosts/); unsupported or unmatched records follow its fallthrough behavior. The configuration uses `/dev/null` for the hosts file to avoid also serving the container's `/etc/hosts`. [CoreDNS 1.12.4 release details](https://github.com/coredns/coredns/releases/tag/v1.12.4) describe the selected release.

Configure the console's DNS resolver, your LAN/VPN DHCP configuration, or a router's conditional forwarding to use the DNS Service's reachable address. An in-cluster `ClusterIP` DNS Service is available by setting `dns.service.type: ClusterIP`; clients outside the cluster then need a separately managed reachable forwarder. Avoid an upstream that points back to this DNS Service, which would create a forwarding loop. The chart requires nonempty `dns.service.loadBalancerSourceRanges` for a DNS LoadBalancer; restrict it to your LAN/VPN and ensure your LoadBalancer honors those ranges. Do not expose it as an unrestricted Internet resolver.

The chart implements this original MAPS TCP bootstrap mapping. It does not invent a MAPS UDP target for the observed original UDP10072 setting: the chart's UDP10474 listener belongs to local DME gameplay, a different service. DNS changes a hostname's address, not a URL scheme, port or path, and does not make an original HTTPS certificate valid for a new endpoint. Set `network.mapsPort` to the port in the original MAPS client endpoint when using a DNS-only redirect; the Service translates it to MAPS container port 10471. This external port must not overlap the other exposed TCP listeners. Other original endpoint port differences require a compatible Service or separately managed port mapping; a hostname override cannot translate ports. The game server still needs the compatible services and private TLS setup for whichever endpoints the client uses. The DNS Service can have its own `dns.service.loadBalancerIP` and controller-specific annotations. Sharing the game LoadBalancer address is controller-specific and requires matching private annotations; it is not assumed by the chart.

## Render, package and update

Required operator values intentionally have no working public defaults: provide a reachable `network.advertisedIPv4`, `network.authDns`, and `runtime.existingSecret` before linting or installing. The checked-in `ci-values.yaml` contains only reserved example addresses and external resource names for offline validation; it does not create the referenced Secret, catalogs or LoadBalancer.

```sh
helm lint charts/mag-recomp-server -f charts/mag-recomp-server/ci-values.yaml
helm template mag charts/mag-recomp-server -f charts/mag-recomp-server/ci-values.yaml
helm package charts/mag-recomp-server --destination /tmp/mag-chart-packages
python3 -m unittest discover -s tests -p test_helm_chart.py
```

The Python graph tests require PyYAML 6.0.3 and Helm 3.19 or newer. They render local manifests, reject invalid values, inspect role/storage/Secret boundaries and the optional DNS graph, and package into a temporary directory. They do not contact a cluster.

Application startup and readiness probes inspect the exact role port in Linux's
`/proc/net/tcp` and `/proc/net/tcp6` tables for TCP state `0A` (LISTEN). They use
`/bin/sh` and `awk`, which must be available in custom images as they are in the
published application images. Either readable address-family table can satisfy
the check; unavailable tables or an absent listener fail the probe. Probe periods
and startup thresholds are unchanged.

These probes open no client connections. The previous TCP socket probes closed
without sending a gameplay request or SCERT handshake, which produced repeated
incomplete-request and disconnect diagnostics despite healthy listeners. Server
diagnostics for actual client traffic are unchanged. The dedicated DNS Pod keeps
its existing HTTP probes.

A listening socket alone does not prove Pod-IP reachability, engine/bridge health,
client login, complete game behavior, data compatibility, external routing, DNS
propagation or load characteristics. The chart binds application listeners to
wildcard addresses; changing that binding needs separate reachability checks.
Test these properties with private operator inputs when deployment is authorized.

When deployment is intended, use your reviewed private values and the chart's independent version:

```sh
helm upgrade --install mag oci://ghcr.io/tekulvw/charts/mag-recomp-server \
  --version 0.1.0 --namespace mag --create-namespace -f /secure/operator-values.yaml
```

The command is an operator example, not a chart hook. No deployment hooks, account migration jobs or cluster-administration resources are installed. Updating public settings, bundle checksum or the bundled scripts changes the Pod template hash and restarts the affected application Pods. For changes to external Secret contents, bump `runtime.revision`; for existing-claim data replacement, bump `data.revision` after preparing consistent data. Neither external Secret rotation nor existing-claim mutation is detected automatically. Recreate updates cause downtime; ephemeral event logs disappear when their Pod is replaced. Preserve any logs you need privately before an update.
