{{/*
Expand the name of the chart.
*/}}
{{- define "oxigraph-cloud.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
*/}}
{{- define "oxigraph-cloud.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{/*
Create chart name and version as used by the chart label.
*/}}
{{- define "oxigraph-cloud.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels.
*/}}
{{- define "oxigraph-cloud.labels" -}}
helm.sh/chart: {{ include "oxigraph-cloud.chart" . }}
{{ include "oxigraph-cloud.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- with .Values.extraLabels }}
{{ toYaml . }}
{{- end }}
{{- end }}

{{/*
Selector labels.
*/}}
{{- define "oxigraph-cloud.selectorLabels" -}}
app.kubernetes.io/name: {{ include "oxigraph-cloud.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
Build the container args based on backend and SHACL configuration.
*/}}
{{- define "oxigraph-cloud.args" -}}
- serve
- --location
- /data/db
- --bind
- 0.0.0.0:7878
{{- if eq .Values.backend "tikv" }}
{{- range .Values.tikv.pdEndpoints }}
- --pd-endpoints
- {{ . | quote }}
{{- end }}
{{- end }}
{{- if and .Values.shacl.mode (ne .Values.shacl.mode "off") }}
- --shacl-mode
- {{ .Values.shacl.mode | quote }}
{{- end }}
{{- end }}

{{/*
Headless service name.
*/}}
{{- define "oxigraph-cloud.headlessServiceName" -}}
{{- printf "%s-headless" (include "oxigraph-cloud.fullname" .) }}
{{- end }}
