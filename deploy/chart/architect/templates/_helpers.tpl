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

{{/* Every path the node owns: node.paths + node.extraPaths. */}}
{{- define "architect.nodePaths" -}}
{{- toJson (concat .Values.node.paths .Values.node.extraPaths) -}}
{{- end -}}
