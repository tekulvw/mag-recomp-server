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
{{ printf "%s:%s" .repository .tag | quote }}
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
{{- define "mag.dataVolume" -}}
- name: data
  {{- if eq .Values.data.mode "s3" }}
  emptyDir:
    sizeLimit: {{ .Values.data.emptyDirSizeLimit }}
  {{- else }}
  persistentVolumeClaim:
    claimName: {{ .Values.data.existingClaim }}
    readOnly: true
  {{- end }}
{{- end -}}
{{- define "mag.dataInit" -}}
{{- if eq .Values.data.mode "s3" }}
- name: fetch-data
  image: {{ include "mag.image" .Values.images.awsCli }}
  imagePullPolicy: {{ .Values.images.awsCli.pullPolicy }}
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
        name: {{ .Values.data.s3.existingSecret }}
  env:
    - {name: AWS_DEFAULT_REGION, value: {{ .Values.data.s3.region | quote }}}
    - {name: AWS_EC2_METADATA_DISABLED, value: "true"}
    - {name: HOME, value: /work}
    - {name: DATA_BUCKET, value: {{ .Values.data.s3.bucket | quote }}}
    - {name: DATA_KEY, value: {{ .Values.data.s3.key | quote }}}
    - {name: DATA_VERSION, value: {{ .Values.data.s3.versionId | quote }}}
    - {name: S3_ENDPOINT, value: {{ .Values.data.s3.endpoint | quote }}}
    - {name: ADDRESSING_STYLE, value: {{ ternary "path" "virtual" .Values.data.s3.forcePathStyle | quote }}}
  securityContext: {allowPrivilegeEscalation: false, capabilities: {drop: [ALL]}}
  resources: {{ toYaml .Values.resources.init | nindent 4 }}
  volumeMounts:
    - {name: archive, mountPath: /download}
    - {name: init-work, mountPath: /work}
- name: verify-data
  image: {{ include "mag.image" .Values.images.python }}
  imagePullPolicy: {{ .Values.images.python.pullPolicy }}
  command: [python3, /scripts/data_bundle.py]
  args: [unpack, --archive, /download/bundle.tar.gz, --sha256, {{ .Values.data.s3.sha256 | quote }}, --output-dir, /catalogs]
  securityContext: {allowPrivilegeEscalation: false, capabilities: {drop: [ALL]}}
  resources: {{ toYaml .Values.resources.init | nindent 4 }}
  volumeMounts:
    - {name: archive, mountPath: /download, readOnly: true}
    - {name: scripts, mountPath: /scripts, readOnly: true}
    - {name: data, mountPath: /catalogs}
{{- end }}
{{- end -}}
