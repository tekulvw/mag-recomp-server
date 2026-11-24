{{- define "mag.name" -}}
{{- printf "%s-mag-recomp-server" .Release.Name | trunc 53 | trimSuffix "-" -}}
{{- end -}}
{{- define "mag.labels" -}}
app.kubernetes.io/name: mag-recomp-server
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | quote }}
{{- end -}}
{{- define "mag.image" -}}
{{- if .digest -}}
{{ printf "%s@%s" .repository .digest | quote }}
{{- else -}}
{{ printf "%s:%s" .repository .tag | quote }}
{{- end -}}
{{- end -}}
{{- define "mag.accountClaim" -}}
{{ default (printf "%s-accounts" (include "mag.name" .)) .Values.accounts.existingClaim }}
{{- end -}}
{{- define "mag.listenerProbe" -}}
exec:
  command:
    - /bin/sh
    - -ec
    - |
      for table in /proc/net/tcp /proc/net/tcp6; do
        if [ -r "$table" ] && awk -v port="$1" '$2 ~ (":" port "$") && $4 == "0A" { found = 1 } END { exit !found }' "$table"; then
          exit 0
        fi
      done
      exit 1
    - socket-listening
    - {{ printf "%04X" (int .port) | quote }}
{{- end -}}
{{- /* A catalog volume: (dict "data" <data.* shape>). */ -}}
{{- define "mag.dataVolume" -}}
- name: data
  {{- if eq .data.mode "s3" }}
  emptyDir:
    sizeLimit: {{ .data.emptyDirSizeLimit }}
  {{- else }}
  persistentVolumeClaim:
    claimName: {{ .data.existingClaim }}
    readOnly: true
  {{- end }}
{{- end -}}
{{- /* Fetch and verify one bundle: (dict "root" $ "data" <data.* shape> "name" "data"|"navigation"). */ -}}
{{- define "mag.dataInit" -}}
{{- $root := .root -}}
{{- $data := .data -}}
{{- $name := .name | default "data" -}}
{{- $archive := ternary "archive" (printf "%s-archive" $name) (eq $name "data") -}}
{{- if eq $data.mode "s3" }}
- name: fetch-{{ $name }}
  image: {{ include "mag.image" $root.Values.images.awsCli }}
  imagePullPolicy: {{ $root.Values.images.awsCli.pullPolicy }}
  command: [/bin/sh, -ec]
  args:
    - |
      mkdir -p /work/aws
      printf '[default]\nregion = %s\ns3 =\n    addressing_style = %s\n' "$AWS_DEFAULT_REGION" "$ADDRESSING_STYLE" > /work/aws/config
      export AWS_CONFIG_FILE=/work/aws/config
      set -- s3api get-object --bucket "$DATA_BUCKET" --key "$DATA_KEY"
      if [ -n "$DATA_VERSION" ]; then set -- "$@" --version-id "$DATA_VERSION"; fi
      if [ -n "$S3_ENDPOINT" ]; then set -- "$@" --endpoint-url "$S3_ENDPOINT"; fi
      aws "$@" /download/bundle.tar.gz >/dev/null
  envFrom:
    - secretRef:
        name: {{ $data.s3.existingSecret }}
  env:
    - {name: AWS_DEFAULT_REGION, value: {{ $data.s3.region | quote }}}
    - {name: AWS_EC2_METADATA_DISABLED, value: "true"}
    - {name: HOME, value: /work}
    - {name: DATA_BUCKET, value: {{ $data.s3.bucket | quote }}}
    - {name: DATA_KEY, value: {{ $data.s3.key | quote }}}
    - {name: DATA_VERSION, value: {{ $data.s3.versionId | quote }}}
    - {name: S3_ENDPOINT, value: {{ $data.s3.endpoint | quote }}}
    - {name: ADDRESSING_STYLE, value: {{ ternary "path" "virtual" $data.s3.forcePathStyle | quote }}}
  securityContext: {allowPrivilegeEscalation: false, capabilities: {drop: [ALL]}}
  resources: {{ toYaml $root.Values.resources.init | nindent 4 }}
  volumeMounts:
    - {name: {{ $archive }}, mountPath: /download}
    - {name: init-work, mountPath: /work}
- name: verify-{{ $name }}
  image: {{ include "mag.image" $root.Values.images.python }}
  imagePullPolicy: {{ $root.Values.images.python.pullPolicy }}
  command: [python3, /scripts/data_bundle.py]
  args: [unpack, --archive, /download/bundle.tar.gz, --sha256, {{ $data.s3.sha256 | quote }}, --output-dir, /catalogs, --replace-existing]
  securityContext: {allowPrivilegeEscalation: false, capabilities: {drop: [ALL]}}
  resources: {{ toYaml $root.Values.resources.init | nindent 4 }}
  volumeMounts:
    - {name: {{ $archive }}, mountPath: /download, readOnly: true}
    - {name: scripts, mountPath: /scripts, readOnly: true}
    - {name: {{ $name }}, mountPath: /catalogs}
{{- end }}
{{- end -}}

{{- define "mag.validateGameplay" -}}
{{- $retail := eq (int .Values.runtime.applicationId) 20231 -}}
{{- $image := .Values.images.gameplay -}}
{{- if and $retail (eq $image.repository "ghcr.io/tekulvw/mag-recomp-server-game") (not $image.digest) (has $image.tag (list "0.1.0" "0.1.1" "v0.1.0" "v0.1.1")) -}}
{{- fail "retail gameplay requires a release containing --application-id (0.2.0 or newer); official 0.1.0/0.1.1 images are incompatible" -}}
{{- end -}}
{{- $rotation := .Values.logs.rotation | default dict -}}
{{- if and $rotation.retentionSeconds (not $rotation.segmentBytes) -}}
{{- fail "logs.rotation.retentionSeconds requires logs.rotation.segmentBytes" -}}
{{- end -}}
{{- $managed := list "--bot-names" "--application-id" "--bind" "--log" "--log-segment-bytes" "--log-retention-s" "--item-catalog" "--spawn-catalog" "--wire-profiles" "--skill-catalog" "--validate-catalogs" "--initial-spawn-delay-ms" -}}
{{- if $retail -}}
{{- $managed = concat $managed (list "--character-blast-rules" "--bootstrap-loadouts" "--retail-combat-metadata" "--retail-armory-catalog" "--fall-damage-rules" "--structure-catalog" "--bunker-catalog" "--bunker-exit-policy") -}}
{{- end -}}
{{- range .Values.data.gameplayExtraArgs -}}
{{- $flag := first (splitList "=" .) -}}
{{- if has $flag $managed -}}
{{- fail (printf "data.gameplayExtraArgs cannot supply chart-managed or unsupported flag %s" $flag) -}}
{{- end -}}
{{- end -}}
{{- $workers := .Values.gameplayWorkers | default list -}}
{{- if $workers -}}
{{- if or (lt (len $workers) 2) (gt (len $workers) 16) -}}
{{- fail "gameplayWorkers needs 2 to 16 entries for the configured world registry; leave it empty for the single gameplay Deployment" -}}
{{- end -}}
{{- if not $retail -}}
{{- fail "gameplayWorkers requires retail runtime.applicationId 20231" -}}
{{- end -}}
{{- $managed = concat $managed (list "--worker-id" "--match-profiles") -}}
{{- $ids := list -}}
{{- $names := list -}}
{{- range $worker := $workers -}}
{{- $name := $worker.name | default $worker.id -}}
{{- if has $worker.id $ids -}}
{{- fail (printf "gameplayWorkers repeats id %s" $worker.id) -}}
{{- end -}}
{{- if has $name $names -}}
{{- fail (printf "gameplayWorkers repeats name %s" $name) -}}
{{- end -}}
{{- $ids = append $ids $worker.id -}}
{{- $names = append $names $name -}}
{{- if gt (len (printf "%s-gameplay-%s" (include "mag.name" $) $name)) 63 -}}
{{- fail (printf "gameplayWorkers %s: the Service name %s-gameplay-%s exceeds 63 characters; set a shorter name" $worker.id (include "mag.name" $) $name) -}}
{{- end -}}
{{- if gt (int ($worker.minimumPlayers | default 2)) (int ($worker.maximumPlayers | default $.Values.runtime.maximumPlayers)) -}}
{{- fail (printf "gameplayWorkers %s: minimumPlayers cannot exceed maximumPlayers" $worker.id) -}}
{{- end -}}
{{- range $worker.extraArgs -}}
{{- $flag := first (splitList "=" .) -}}
{{- if has $flag $managed -}}
{{- fail (printf "gameplayWorkers %s extraArgs cannot supply chart-managed or unsupported flag %s" $worker.id $flag) -}}
{{- end -}}
{{- end -}}
{{- end -}}
{{- end -}}
{{- end -}}

{{- /*
Collect the Deployments to render into .pods: one entry per configured gameplay worker (or the
single gameplay Pod when gameplayWorkers is empty), then Medius. A worker's data and navigation
bundles inherit only the object-store location (endpoint, region, bucket, existingSecret,
forcePathStyle) from data.s3; key, sha256 and versionId always belong to the worker.
*/ -}}
{{- define "mag.workerBundle" -}}
{{- $s3 := deepCopy .root.Values.data.s3 -}}
{{- $_ := set $s3 "versionId" "" -}}
{{- range $key, $value := .bundle.s3 | default dict -}}
{{- $_ := set $s3 $key $value -}}
{{- end -}}
{{- range $key := list "bucket" "key" "sha256" "existingSecret" -}}
{{- if not (index $s3 $key) -}}
{{- fail (printf "gameplayWorkers %s needs %s.s3.%s (or data.s3.%s to inherit the store)" $.id $.field $key $key) -}}
{{- end -}}
{{- end -}}
{{- $_ := set .out "s3" $s3 -}}
{{- end -}}
{{- define "mag.pods" -}}
{{- $root := .root -}}
{{- $values := $root.Values -}}
{{- $pods := list -}}
{{- $global := dict "nodeSelector" $values.nodeSelector "tolerations" $values.tolerations "affinity" $values.affinity -}}
{{- range $worker := $values.gameplayWorkers | default list -}}
{{- $workerData := $worker.data | default dict -}}
{{- $bundle := dict -}}
{{- $_ := include "mag.workerBundle" (dict "root" $root "bundle" $workerData "out" $bundle "id" $worker.id "field" "data") -}}
{{- $data := dict "mode" "s3" "s3" $bundle.s3 "emptyDirSizeLimit" ($worker.emptyDirSizeLimit | default $values.data.emptyDirSizeLimit) "revision" ($workerData.revision | default "1") -}}
{{- $files := deepCopy $values.data.files -}}
{{- range $key, $value := $workerData.files | default dict -}}
{{- $_ := set $files $key $value -}}
{{- end -}}
{{- $navigation := dict -}}
{{- with $worker.navigation -}}
{{- $navigationBundle := dict -}}
{{- $_ := include "mag.workerBundle" (dict "root" $root "bundle" . "out" $navigationBundle "id" $worker.id "field" "navigation") -}}
{{- $navigation = dict "mode" "s3" "s3" $navigationBundle.s3 "emptyDirSizeLimit" (.emptyDirSizeLimit | default $values.data.emptyDirSizeLimit) "revision" (.revision | default "1") "mountPath" (.mountPath | default "/navigation") -}}
{{- end -}}
{{- $pod := dict "component" (printf "gameplay-%s" ($worker.name | default $worker.id)) "worker" $worker "data" $data "files" $files "extraArgs" ($worker.extraArgs | default list) "navigation" $navigation "resources" ($worker.resources | default $values.resources.gameplay) -}}
{{- range $key, $value := $global -}}
{{- $_ := set $pod $key (ternary (index $worker $key) $value (hasKey $worker $key)) -}}
{{- end -}}
{{- $pods = append $pods $pod -}}
{{- end -}}
{{- if not $pods -}}
{{- $pods = append $pods (merge (dict "component" "gameplay" "data" $values.data "files" $values.data.files "extraArgs" $values.data.gameplayExtraArgs "resources" $values.resources.gameplay) $global) -}}
{{- end -}}
{{- $pods = append $pods (merge (dict "component" "medius" "data" $values.data "files" $values.data.files) $global) -}}
{{- $_ := set . "pods" $pods -}}
{{- end -}}
{{- /* Medius UDP ports after 10474, as a JSON array; the integrated DME always starts at 10474. */ -}}
{{- define "mag.extraUdpPorts" -}}
{{- $range := .Values.network.udpPortRange | default dict -}}
{{- $start := int ($range.start | default 10474) -}}
{{- $end := int ($range.end | default 10474) -}}
{{- if ne $start 10474 -}}
{{- fail "network.udpPortRange.start must be 10474: the integrated DME hands out UDP ports from 10474 upward" -}}
{{- end -}}
{{- if lt $end $start -}}
{{- fail "network.udpPortRange.end cannot be below its start" -}}
{{- end -}}
{{- toJson (untilStep 10475 (int (add1 $end)) 1) -}}
{{- end -}}
{{- /*
The configured world registry (docs/configured-world-registry.md) as JSON: (dict "root" $ "pods" <mag.pods>
"structure" bool). Medius reads client_address from MAG_ADVERTISE_IPV4, so an operator that keeps that
variable current needs no second copy of the address. "structure" clears drain flags, giving the
checksum of everything Medius cannot reload live.
*/ -}}
{{- define "mag.worldRegistry" -}}
{{- $root := .root -}}
{{- $structure := .structure -}}
{{- $rows := list -}}
{{- range $pod := .pods -}}
{{- with $pod.worker -}}
{{- $teams := list -}}
{{- range .minimumPlayersPerTeam | default (list 1 1) -}}
{{- $teams = append $teams (int .) -}}
{{- end -}}
{{- $draining := and (not $structure) (.draining | default false) -}}
{{- $rows = append $rows (dict "worker_id" .id "worker_host" (printf "%s-%s" (include "mag.name" $root) $pod.component) "worker_port" 31000 "minimum_players" (int (.minimumPlayers | default 2)) "maximum_players" (int (.maximumPlayers | default $root.Values.runtime.maximumPlayers)) "minimum_players_per_team" $teams "draining" $draining) -}}
{{- end -}}
{{- end -}}
{{- $registry := dict "format" "mag_world_registry_v1" "application_id" 20231 "client_address" "$MAG_ADVERTISE_IPV4" "client_tcp_port" 10473 "workers" $rows -}}
{{- with ($root.Values.worldRegistry | default dict).partyInviteSeconds -}}
{{- $_ := set $registry "party_invite_seconds" (int .) -}}
{{- end -}}
{{- toPrettyJson $registry -}}
{{- end -}}
