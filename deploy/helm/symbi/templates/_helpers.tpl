{{- define "symbi.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "symbi.fullname" -}}
{{- printf "%s-%s" .Release.Name (include "symbi.name" .) | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "symbi.labels" -}}
app.kubernetes.io/name: {{ include "symbi.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" }}
{{- end -}}

{{- define "symbi.selectorLabels" -}}
app.kubernetes.io/name: {{ include "symbi.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "symbi.secretName" -}}
{{- if .Values.existingSecret -}}{{ .Values.existingSecret }}{{- else -}}{{ include "symbi.fullname" . }}{{- end -}}
{{- end -}}
