{{- /* One Deployment: (dict "root" $ "pod" <entry from mag.pods> "pods" <all entries>). */ -}}
{{- define "mag.deployment" -}}
{{- $root := .root }}
{{- $pod := .pod }}
{{- $pods := .pods }}
{{- $component := $pod.component }}
{{- $data := $pod.data }}
{{- $files := $pod.files }}
{{- $worker := $pod.worker }}
{{- $medius := eq $component "medius" }}
{{- $portal := $root.Values.portalTickets | default dict }}
{{- $ownerLookup := $portal.accountLookup | default dict }}
{{- $ownersRequired := eq ($ownerLookup.mode | default "off") "required" }}
{{- if and $medius $ownersRequired }}
{{- if or (ne ($portal.mode | default "off") "required") (not $ownerLookup.existingSecret) (not $ownerLookup.secretKey) }}
{{- fail "portalTickets.accountLookup=required needs portalTickets.mode=required, accountLookup.existingSecret and accountLookup.secretKey" }}
{{- end }}
{{- end }}
{{- $registry := and $medius (not (empty $root.Values.gameplayWorkers)) }}
{{- $shutdown := $root.Values.gameplayShutdown | default dict }}
{{- $graceful := and (not $medius) (or (not (kindIs "invalid" $shutdown.drainSeconds)) (not (kindIs "invalid" $shutdown.receiptFlushSeconds))) }}
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: {{ include "mag.name" $root }}-{{ $component }}
  labels:
    {{- include "mag.labels" $root | nindent 4 }}
    app.kubernetes.io/component: {{ $component }}
spec:
  replicas: 1
  strategy: {type: Recreate}
  selector:
    matchLabels:
      app.kubernetes.io/instance: {{ $root.Release.Name }}
      app.kubernetes.io/component: {{ $component }}
  template:
    metadata:
      labels:
        app.kubernetes.io/name: mag-recomp-server
        app.kubernetes.io/instance: {{ $root.Release.Name }}
        app.kubernetes.io/component: {{ $component }}
      annotations:
        checksum/config: {{ include (print $root.Template.BasePath "/configmap.yaml") $root | sha256sum }}
        runtime-revision: {{ $root.Values.runtime.revision | quote }}
        data-revision: {{ $data.revision | quote }}
        {{- with $pod.navigation }}
        navigation-revision: {{ .revision | quote }}
        {{- end }}
        {{- if $medius }}
        checksum/news-image: {{ include "mag.image" $root.Values.images.gameplay | sha256sum }}
        {{- end }}
        {{- if $registry }}
        {{- /* Drain flags reload live; any other registry change, and any change that rolls a
               worker, must also rebuild Medius because it never adopts a restarted worker. */}}
        checksum/world-registry: {{ include "mag.worldRegistry" (dict "root" $root "pods" $pods "structure" true) | sha256sum }}
        {{- range $other := $pods }}
        {{- if $other.worker }}
        checksum/{{ $other.component }}: {{ include "mag.deployment" (dict "root" $root "pod" $other "pods" $pods) | sha256sum }}
        {{- end }}
        {{- end }}
        {{- end }}
    spec:
      automountServiceAccountToken: false
      {{- if $graceful }}
      {{- /* The worker drains on SIGTERM; give it the whole drain, its receipt flush and a margin. */}}
      terminationGracePeriodSeconds: {{ add (int64 ($shutdown.drainSeconds | default 0)) (int64 (ternary 5 $shutdown.receiptFlushSeconds (kindIs "invalid" $shutdown.receiptFlushSeconds))) 15 }}
      {{- end }}
      securityContext:
        runAsNonRoot: true
        runAsUser: 10001
        runAsGroup: 10001
        fsGroup: 10001
        fsGroupChangePolicy: OnRootMismatch
        seccompProfile: {type: RuntimeDefault}
      {{- with $root.Values.imagePullSecrets }}
      imagePullSecrets: {{ toYaml . | nindent 8 }}
      {{- end }}
      nodeSelector: {{ toYaml $pod.nodeSelector | nindent 8 }}
      {{- with $pod.tolerations }}
      tolerations: {{ toYaml . | nindent 8 }}
      {{- end }}
      {{- with $pod.affinity }}
      affinity: {{ toYaml . | nindent 8 }}
      {{- end }}
      {{- if or $medius (eq $data.mode "s3") }}
      initContainers:
        {{- include "mag.dataInit" (dict "root" $root "data" $data) | nindent 8 }}
        {{- with $pod.navigation }}
        {{- include "mag.dataInit" (dict "root" $root "data" . "name" "navigation") | nindent 8 }}
        {{- end }}
        {{- if $medius }}
        - name: prepare-news
          image: {{ include "mag.image" $root.Values.images.gameplay }}
          imagePullPolicy: {{ $root.Values.images.gameplay.pullPolicy }}
          command: [/bin/sh, -ec]
          args:
            - |
              rm -f /news/gameplay-CHANGELOG.md
              if [ -f /usr/share/mag-recomp-server/gameplay-CHANGELOG.md ]; then
                cp /usr/share/mag-recomp-server/gameplay-CHANGELOG.md /news/gameplay-CHANGELOG.md
              fi
          securityContext: {allowPrivilegeEscalation: false, capabilities: {drop: [ALL]}}
          resources: {{ toYaml $root.Values.resources.init | nindent 12 }}
          volumeMounts:
            - {name: news, mountPath: /news}
        - name: prepare-config
          image: {{ include "mag.image" $root.Values.images.python }}
          imagePullPolicy: {{ $root.Values.images.python.pullPolicy }}
          command: [python3, /scripts/init_config.py]
          args: [/public-config, /runtime-secrets, /config]
          securityContext: {allowPrivilegeEscalation: false, capabilities: {drop: [ALL]}}
          resources: {{ toYaml $root.Values.resources.init | nindent 12 }}
          volumeMounts:
            - {name: scripts, mountPath: /scripts, readOnly: true}
            - {name: public-config, mountPath: /public-config, readOnly: true}
            - {name: runtime-secrets, mountPath: /runtime-secrets, readOnly: true}
            - {name: config, mountPath: /config}
        {{- $wait := int ($root.Values.worldRegistry | default dict).workerWaitSeconds }}
        {{- if and $registry (gt $wait 0) }}
        - name: wait-for-workers
          image: {{ include "mag.image" $root.Values.images.python }}
          imagePullPolicy: {{ $root.Values.images.python.pullPolicy }}
          command: [python3, /scripts/wait_for_workers.py]
          args:
            - {{ $wait | toString | quote }}
            {{- range $other := $pods }}
            {{- if $other.worker }}
            - {{ printf "%s-%s:31000" (include "mag.name" $root) $other.component | quote }}
            {{- end }}
            {{- end }}
          securityContext: {allowPrivilegeEscalation: false, capabilities: {drop: [ALL]}}
          resources: {{ toYaml $root.Values.resources.init | nindent 12 }}
          volumeMounts:
            - {name: scripts, mountPath: /scripts, readOnly: true}
        {{- end }}
        {{- end }}
      {{- end }}
      containers:
        {{- if $medius }}
        {{- range $role := list "medius" "maps" }}
        {{- $main := eq $role "medius" }}
        - name: {{ $role }}
          image: {{ include "mag.image" $root.Values.images.medius }}
          imagePullPolicy: {{ $root.Values.images.medius.pullPolicy }}
          workingDir: /app/medius
          command: [/bin/sh, -ec]
          args:
            - |
              mkdir -p /logs/{{ ternary "main" "maps" $main }}
              exec dotnet /app/medius/Server.Medius.dll /config/{{ ternary "main" "maps" $main }}
          env:
            - {name: HORIZON_SCERT_MAS_CERTIFICATE_PATH, value: /runtime-secrets/scert-mas-certificate.bin}
            - {name: HORIZON_SCERT_MLS_CERTIFICATE_PATH, value: /runtime-secrets/scert-mls-certificate.bin}
            - {name: HOME, value: /config}
            - {name: DOTNET_CLI_HOME, value: /config}
            - {name: MAG_ADVERTISE_IPV4, value: {{ $root.Values.network.advertisedIPv4 | quote }}}
            - {name: MAG_AUTH_DNS, value: {{ $root.Values.network.authDns | quote }}}
            - {name: MAG_AUTH_PORT, value: "10475"}
            - {name: MAG_LOBBY_PORT, value: "10478"}
            - {name: MAG_GAME_TCP_PORT, value: "10473"}
            - {name: MAG_GAMEPLAY_NEWS_PATH, value: /news/gameplay-CHANGELOG.md}
            {{- if $main }}
            - {name: MAG_LOCAL_DME, value: "1"}
            {{- /* One admitted human per forwarded DME UDP port, across every world. */}}
            - {name: MAG_DME_UDP_PORT_END, value: {{ include "mag.udpPortEnd" $root | quote }}}
            {{- with $root.Values.network.dmeHumanCapacity }}
            {{- if gt (int .) (int (include "mag.udpPortCount" $root)) }}
            {{- fail "network.dmeHumanCapacity cannot exceed the ports in network.udpPortRange" }}
            {{- end }}
            - {name: MAG_DME_HUMAN_CAPACITY, value: {{ . | int | toString | quote }}}
            {{- end }}
            {{- if $registry }}
            - {name: MAG_WORLD_REGISTRY, value: /world-registry/world-registry.json}
            {{- else }}
            - {name: MAG_GAMEPLAY_HOST, value: {{ printf "%s-gameplay" (include "mag.name" $root) | quote }}}
            - {name: MAG_GAMEPLAY_PORT, value: "31000"}
            {{- end }}
            {{- end }}
            {{- $catalogs := dict "MAG_TRAINING_COMPLETION_CATALOG" "trainingCompletion" "MAG_PROGRESSION_CATALOG" "progression" }}
            {{- if eq (int $root.Values.runtime.applicationId) 20231 }}
            {{- $_ := set $catalogs "MAG_RETAIL_SKILL_CATALOG" "retailSkills" }}
            {{- $_ := set $catalogs "MAG_RETAIL_ARMORY_CATALOG" "retailArmory" }}
            {{- end }}
            {{- range $env, $file := $catalogs }}
            {{- with index $root.Values.data.files $file }}
            - {name: {{ $env }}, value: {{ printf "/data/%s" . | quote }}}
            {{- end }}
            {{- end }}
            {{- with $root.Values.runtime.startingLevel }}
            - {name: MAG_STARTING_LEVEL, value: {{ . | int | toString | quote }}}
            {{- end }}
            {{- with $root.Values.runtime.trainingLevel }}
            - {name: MAG_TRAINING_LEVEL, value: {{ . | int | toString | quote }}}
            {{- end }}
            {{- with $root.Values.runtime.respecPointsFloor }}
            - {name: MAG_RESPEC_POINTS_FLOOR, value: {{ . | int64 | toString | quote }}}
            {{- end }}
            {{- if ne $root.Values.runtime.respecFeeCap nil }}
            - {name: MAG_RESPEC_FEE_CAP, value: {{ $root.Values.runtime.respecFeeCap | int64 | toString | quote }}}
            {{- end }}
            {{- with $root.Values.runtime.rewardLedgerOpenMatchTtlHours }}
            - {name: MAG_REWARD_LEDGER_OPEN_MATCH_TTL_HOURS, value: {{ . | int | toString | quote }}}
            {{- end }}
            {{- with (include "mag.characterDatabase" $root | fromJson) }}
            {{- if eq .backend "postgres" }}
            - {name: MAG_CHARACTER_STORE, value: postgres}
            {{- end }}
            - {name: MAG_CHARACTER_DB_SECRET_DIR, value: /character-db}
            {{- if .caSecret }}
            - {name: MAG_CHARACTER_DB_CA_FILE, value: /character-db-ca/ca.crt}
            {{- end }}
            {{- if not $main }}
            - {name: MAG_CHARACTER_DB_READ_ONLY, value: "1"}
            {{- end }}
            {{- with .ledgerRetentionDays }}
            - {name: MAG_REWARD_LEDGER_RETENTION_DAYS, value: {{ . | int | toString | quote }}}
            {{- end }}
            {{- end }}
            {{- if eq ($portal.mode | default "off") "required" }}
            {{- $keys := list }}
            {{- range $id, $spki := $portal.keys }}
            {{- $keys = append $keys (printf "%s:%s" $id $spki) }}
            {{- end }}
            {{- if or (not $portal.audience) (empty $keys) }}
            {{- fail "portalTickets.mode=required needs portalTickets.audience and at least one portalTickets.keys entry" }}
            {{- end }}
            - {name: MAG_PORTAL_TICKETS, value: required}
            - {name: MAG_PORTAL_TICKET_AUDIENCE, value: {{ $portal.audience | quote }}}
            - {name: MAG_PORTAL_TICKET_KEYS, value: {{ join "," $keys | quote }}}
            {{- if $ownersRequired }}
            - {name: MAG_PORTAL_ACCOUNT_LOOKUP, value: required}
            - {name: MAG_PORTAL_ACCOUNT_DB_CONNECTION_FILE, value: /portal-account-db/connection-string}
            {{- end }}
            {{- end }}
            {{- if and $main $root.Values.adminApi.enabled }}
            - {name: MAG_ADMIN_API_LISTEN, value: {{ printf "0.0.0.0:%d" (int $root.Values.adminApi.port) | quote }}}
            - {name: MAG_ADMIN_API_SECRET_FILE, value: /admin-api/secret}
            {{- end }}
          ports:
            {{- if $main }}
            - {name: auth, containerPort: 10475, protocol: TCP}
            - {name: lobby, containerPort: 10478, protocol: TCP}
            - {name: proxy, containerPort: 10477, protocol: TCP}
            - {name: game-tcp, containerPort: 10073, protocol: TCP}
            - {name: game-udp, containerPort: 10474, protocol: UDP}
            {{- range $port := include "mag.extraUdpPorts" $root | fromJsonArray }}
            - {name: {{ printf "game-udp-%d" (int $port) }}, containerPort: {{ int $port }}, protocol: UDP}
            {{- end }}
            {{- if $root.Values.adminApi.enabled }}
            - {name: admin-api, containerPort: {{ int $root.Values.adminApi.port }}, protocol: TCP}
            {{- end }}
            {{- else }}
            - {name: maps-auth, containerPort: 10471, protocol: TCP}
            - {name: maps-lobby, containerPort: 10472, protocol: TCP}
            - {name: maps-proxy, containerPort: 10476, protocol: TCP}
            {{- end }}
          startupProbe:
            {{- include "mag.listenerProbe" (dict "port" (ternary 10475 10471 $main)) | nindent 12 }}
            periodSeconds: 2
            failureThreshold: 90
          readinessProbe:
            {{- include "mag.listenerProbe" (dict "port" (ternary 10475 10471 $main)) | nindent 12 }}
            periodSeconds: 5
          securityContext: {allowPrivilegeEscalation: false, capabilities: {drop: [ALL]}}
          resources: {{ toYaml (index $root.Values.resources $role) | nindent 12 }}
          volumeMounts:
            - {name: news, mountPath: /news, readOnly: true}
            - {name: data, mountPath: /data, readOnly: true}
            - {name: accounts, mountPath: /accounts, readOnly: {{ not $main }}}
            - {name: config, mountPath: /config}
            - {name: runtime-secrets, mountPath: /runtime-secrets, readOnly: true}
            - {name: logs, mountPath: /logs}
            - {name: tmp, mountPath: /tmp}
            {{- if $ownersRequired }}
            - {name: portal-account-db, mountPath: /portal-account-db, readOnly: true}
            {{- end }}
            {{- if and $main $root.Values.adminApi.enabled }}
            - {name: admin-api-secret, mountPath: /admin-api, readOnly: true}
            {{- end }}
            {{- if and $main $registry }}
            - {name: world-registry, mountPath: /world-registry, readOnly: true}
            {{- end }}
            {{- with (include "mag.characterDatabase" $root | fromJson) }}
            - {name: character-db, mountPath: /character-db, readOnly: true}
            {{- if .caSecret }}
            - {name: character-db-ca, mountPath: /character-db-ca, readOnly: true}
            {{- end }}
            {{- end }}
        {{- end }}
        {{- else }}
        - name: gameplay
          image: {{ include "mag.image" $root.Values.images.gameplay }}
          imagePullPolicy: {{ $root.Values.images.gameplay.pullPolicy }}
          command: [/bin/sh, -ec]
          args:
            - |
              run_dir=$(mktemp -d /logs/run.XXXXXXXXXX)
              exec /usr/local/bin/mag-gameplay --bind 0.0.0.0:31000 --log "$run_dir/events.jsonl"{{ with $root.Values.logs.rotation }}{{ if .segmentBytes }} --log-segment-bytes {{ int64 .segmentBytes }}{{ end }}{{ if .retentionSeconds }} --log-retention-s {{ int64 .retentionSeconds }}{{ end }}{{ end }} --item-catalog "$ITEMS" --spawn-catalog "$SPAWNS"{{ with $root.Values.runtime.initialSpawnDelayMs }} --initial-spawn-delay-ms {{ if kindIs "slice" . }}{{ index . 0 | int64 }},{{ index . 1 | int64 }}{{ else }}{{ int64 . }}{{ end }}{{ end }} --wire-profiles "$WIRES" --skill-catalog "$SKILLS"{{ if $files.botNames }} --bot-names "$BOT_NAMES"{{ end }}{{ if eq (int $root.Values.runtime.applicationId) 20231 }} --application-id 20231 --character-blast-rules "$BLAST" --bootstrap-loadouts "$BOOTSTRAP" --retail-combat-metadata "$COMBAT" --retail-armory-catalog "$ARMORY" --fall-damage-rules "$FALL"{{ end }}{{ if $worker }} --worker-id "$WORKER_ID" --match-profiles "$MATCH_PROFILES"{{ end }}{{ with $shutdown }}{{ if not (kindIs "invalid" .drainSeconds) }} --drain-seconds {{ int64 .drainSeconds }}{{ end }}{{ if not (kindIs "invalid" .receiptFlushSeconds) }} --receipt-flush-seconds {{ int64 .receiptFlushSeconds }}{{ end }}{{ end }} "$@"
            - mag-gameplay
            {{- range $pod.extraArgs }}
            - {{ . | quote }}
            {{- end }}
          env:
            {{- with $files.botNames }}
            - {name: BOT_NAMES, value: {{ printf "/data/%s" . | quote }}}
            {{- end }}
            - {name: ITEMS, value: {{ printf "/data/%s" $files.items | quote }}}
            - {name: SPAWNS, value: {{ printf "/data/%s" $files.spawns | quote }}}
            - {name: WIRES, value: {{ printf "/data/%s" $files.wireProfiles | quote }}}
            - {name: SKILLS, value: {{ printf "/data/%s" $files.skills | quote }}}
            {{- if eq (int $root.Values.runtime.applicationId) 20231 }}
            {{- range $env, $file := dict "BLAST" "characterBlastRules" "BOOTSTRAP" "bootstrapLoadouts" "COMBAT" "retailCombatMetadata" "ARMORY" "retailArmory" "FALL" "fallDamageRules" }}
            - {name: {{ $env }}, value: {{ printf "/data/%s" (index $files $file) | quote }}}
            {{- end }}
            {{- end }}
            {{- with $worker }}
            - {name: WORKER_ID, value: {{ .id | quote }}}
            - {name: MATCH_PROFILES, value: {{ printf "/data/%s" .matchProfiles | quote }}}
            {{- end }}
          ports:
            - {name: bridge, containerPort: 31000, protocol: TCP}
          startupProbe:
            {{- include "mag.listenerProbe" (dict "port" 31000) | nindent 12 }}
            periodSeconds: 2
            failureThreshold: 90
          readinessProbe:
            {{- include "mag.listenerProbe" (dict "port" 31000) | nindent 12 }}
            periodSeconds: 5
          securityContext: {allowPrivilegeEscalation: false, capabilities: {drop: [ALL]}}
          resources: {{ toYaml $pod.resources | nindent 12 }}
          volumeMounts:
            - {name: data, mountPath: /data, readOnly: true}
            - {name: logs, mountPath: /logs}
            - {name: tmp, mountPath: /tmp}
            {{- with $pod.navigation }}
            - {name: navigation, mountPath: {{ .mountPath }}, readOnly: true}
            {{- end }}
        {{- end }}
      volumes:
        {{- include "mag.dataVolume" (dict "data" $data) | nindent 8 }}
        - name: logs
          emptyDir: {sizeLimit: {{ $root.Values.logs.sizeLimit }}}
        - name: tmp
          emptyDir: {}
        {{- if or $medius (eq $data.mode "s3") }}
        - name: scripts
          configMap:
            name: {{ include "mag.name" $root }}-scripts
        {{- end }}
        {{- if eq $data.mode "s3" }}
        - name: archive
          emptyDir: {sizeLimit: {{ $data.emptyDirSizeLimit }}}
        - name: init-work
          emptyDir: {}
        {{- end }}
        {{- with $pod.navigation }}
        - name: navigation
          emptyDir: {sizeLimit: {{ .emptyDirSizeLimit }}}
        - name: navigation-archive
          emptyDir: {sizeLimit: {{ .emptyDirSizeLimit }}}
        {{- end }}
        {{- if $medius }}
        {{- if $ownersRequired }}
        - name: portal-account-db
          secret:
            secretName: {{ $ownerLookup.existingSecret | quote }}
            items:
              - {key: {{ $ownerLookup.secretKey | quote }}, path: connection-string}
            defaultMode: 0440
        {{- end }}
        - name: news
          emptyDir: {sizeLimit: 2Mi}
        - name: accounts
          persistentVolumeClaim:
            claimName: {{ include "mag.accountClaim" $root }}
        - name: config
          emptyDir: {}
        - name: public-config
          configMap:
            name: {{ include "mag.name" $root }}-config
        - name: runtime-secrets
          secret:
            secretName: {{ $root.Values.runtime.existingSecret }}
            defaultMode: 0440
        {{- if $root.Values.adminApi.enabled }}
        - name: admin-api-secret
          secret:
            secretName: {{ $root.Values.adminApi.existingSecret }}
            items:
              - {key: {{ $root.Values.adminApi.secretKey | quote }}, path: secret}
            defaultMode: 0440
        {{- end }}
        {{- if $registry }}
        - name: world-registry
          configMap:
            name: {{ include "mag.name" $root }}-world-registry
        {{- end }}
        {{- with (include "mag.characterDatabase" $root | fromJson) }}
        - name: character-db
          secret:
            secretName: {{ .existingSecret }}
            items:
              {{- range $key := list "host" "port" "dbname" "user" "password" }}
              - {key: {{ $key }}, path: {{ $key }}}
              {{- end }}
            defaultMode: 0440
        {{- if .caSecret }}
        - name: character-db-ca
          secret:
            secretName: {{ .caSecret }}
            items:
              - {key: ca.crt, path: ca.crt}
            defaultMode: 0440
        {{- end }}
        {{- end }}
        {{- end }}
{{- end -}}
