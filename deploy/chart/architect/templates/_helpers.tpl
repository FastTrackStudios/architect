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

{{/* A Kubernetes quantity (10Gi, 500M, 1T, 2048) in bytes. */}}
{{- define "architect.bytes" -}}
{{- $q := toString . -}}
{{- $num := regexFind "^[0-9]+" $q -}}
{{- if not $num }}{{ fail (printf "%q is not a size" $q) }}{{ end -}}
{{- $unit := trimPrefix $num $q -}}
{{- $mult := get (dict "" 1 "k" 1000 "K" 1000 "M" 1000000 "G" 1000000000 "T" 1000000000000 "Ki" 1024 "Mi" 1048576 "Gi" 1073741824 "Ti" 1099511627776) $unit -}}
{{- if not $mult }}{{ fail (printf "%q: unknown unit %q" $q $unit) }}{{ end -}}
{{- mul (int64 $num) (int64 $mult) -}}
{{- end -}}

{{/* The node's store limit: node.store.max, else 75% of its data volume. */}}
{{- define "architect.storeMax" -}}
{{- $n := .Values.node -}}
{{- if $n.store.max -}}
{{- $n.store.max -}}
{{- else -}}
{{- $volume := ternary $n.persistence.size $n.persistence.emptyDirSizeLimit $n.persistence.enabled -}}
{{- div (mul (include "architect.bytes" $volume | int64) 3) 4 -}}
{{- end -}}
{{- end -}}

{{/* Every path the node owns: node.paths + node.extraPaths. */}}
{{- define "architect.nodePaths" -}}
{{- toJson (concat .Values.node.paths .Values.node.extraPaths) -}}
{{- end -}}
