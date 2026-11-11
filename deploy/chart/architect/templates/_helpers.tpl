{{- define "architect.fullname" -}}
{{- if contains .Chart.Name .Release.Name -}}{{ .Release.Name | trunc 63 | trimSuffix "-" }}{{- else -}}{{ printf "%s-%s" .Release.Name .Chart.Name | trunc 63 | trimSuffix "-" }}{{- end -}}
{{- end -}}

{{- define "architect.labels" -}}
app.kubernetes.io/name: {{ .Chart.Name }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version }}
{{- end -}}

{{- define "architect.selector" -}}
app.kubernetes.io/name: {{ .root.Chart.Name }}
app.kubernetes.io/instance: {{ .root.Release.Name }}
app.kubernetes.io/component: {{ .component }}
{{- end -}}

{{/* A repository with a "/" is a full name, used as is (argocd-image-updater
     writes ghcr.io/fasttrackstudios/architect); a bare one gets image.registry. */}}
{{- define "architect.image" -}}
{{- $repo := .image.repository -}}
{{- if not (contains "/" $repo) }}{{ $repo = printf "%s/%s" .root.Values.image.registry $repo }}{{ end -}}
{{ $repo }}:{{ .image.tag | default .root.Chart.AppVersion }}
{{- end -}}

{{/* A Kubernetes quantity (10Gi, 1.5Gi, 500M, 2048) in bytes; fails on anything else. */}}
{{- define "architect.bytes" -}}
{{- $q := toString . -}}
{{- $num := regexFind "^[0-9]+(\\.[0-9]+)?" $q -}}
{{- if not $num }}{{ fail (printf "%q is not a size (bytes, 16Gi, 1.5Gi)" $q) }}{{ end -}}
{{- $unit := trimPrefix $num $q -}}
{{- $mult := get (dict "" 1 "k" 1000 "K" 1000 "M" 1000000 "G" 1000000000 "T" 1000000000000 "Ki" 1024 "Mi" 1048576 "Gi" 1073741824 "Ti" 1099511627776) $unit -}}
{{- if not $mult }}{{ fail (printf "%q: unknown unit %q" $q $unit) }}{{ end -}}
{{- if contains "." $num -}}
{{- mulf (float64 $num) (float64 $mult) | floor | int64 -}}
{{- else -}}
{{- mul (int64 $num) (int64 $mult) -}}
{{- end -}}
{{- end -}}

{{/* The data volume's size in bytes: the PVC's, else the emptyDir's limit. */}}
{{- define "architect.dataBytes" -}}
{{- $p := .Values.node.persistence -}}
{{- include "architect.bytes" (ternary $p.size $p.emptyDirSizeLimit $p.enabled) -}}
{{- end -}}

{{/* The node's store limit in bytes: node.store.max (validated), else
     75% of its data volume. It must leave room on the volume. */}}
{{- define "architect.storeMax" -}}
{{- $n := .Values.node -}}
{{- $volume := include "architect.dataBytes" . | int64 -}}
{{- if eq (toString $n.store.max) "0" }}{{ fail "node.store.max must be more than 0 (empty is 75% of the data volume)" }}{{ end -}}
{{- if $n.store.max -}}
{{- $max := include "architect.bytes" $n.store.max | int64 -}}
{{- if le $max 0 }}{{ fail "node.store.max must be more than 0" }}{{ end -}}
{{- if gt $max $volume }}{{ fail (printf "node.store.max (%d bytes) is more than the data volume (%d bytes)" $max $volume) }}{{ end -}}
{{- $max -}}
{{- else -}}
{{- div (mul $volume 3) 4 -}}
{{- end -}}
{{- end -}}

{{/* The container's ephemeral-storage limit: node.ephemeralStorage, plus
     the emptyDir /data when persistence is off (the kubelet counts an
     emptyDir against it, so the two must agree). */}}
{{- define "architect.ephemeralLimit" -}}
{{- $n := .Values.node -}}
{{- $own := include "architect.bytes" $n.ephemeralStorage | int64 -}}
{{- if $n.persistence.enabled -}}
{{- $own -}}
{{- else -}}
{{- add $own (include "architect.dataBytes" . | int64) -}}
{{- end -}}
{{- end -}}

{{/* Every path the node owns: node.paths + node.extraPaths. */}}
{{- define "architect.nodePaths" -}}
{{- toJson (concat .Values.node.paths .Values.node.extraPaths) -}}
{{- end -}}
