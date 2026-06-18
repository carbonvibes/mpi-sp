package template

import (
	"bytes"
	_ "embed"
	"fmt"
	"go/format"
	"strings"
	"text/template"
)

var (
	//go:embed templates/sanitizer.go.in
	sanitizerGoTemplate string
	//go:embed templates/sanitizer.bpf.c.in
	sanitizerBPFCTemplate string
	//go:embed templates/generate.go.in
	sanitizerGoGenerateTemplate string
)

type SanitizerTemplateData struct {
	// Name is the name of the sanitizer.
	Name string
	// BindingStructs is the list of binding structs to genearte.
	BindingStructs []BindingStruct
	// Maps is the list of BPF maps to generate.
	Maps []BPFMap
	// Tracepoints is the map of tracepoints to attach probes to.
	Tracepoints map[string]Tracepoint
	// Kprobes is the map of kprobes to attach.
	Kprobes map[string]Kprobe
}

// BPFMap holds the specification for a BPF map.
type BPFMap struct {
	// Name is the name of the map.
	Name string
	// Type is the BPF map type (BPF_MAP_TYPE_HASH or BPF_MAP_TYPE_ARRAY).
	Type string
	// KeyType is the C type of the map key.
	KeyType string
	// ValueType is the C type of the map value.
	ValueType string
	// MaxEntries is the maximum number of entries in the map.
	MaxEntries int
}

// BindingStruct holds the Go and C definitions of a binding struct.
type BindingStruct struct {
	// GoDefinition is the Go struct definition.
	GoDefinition string
	// CDefinition is the C struct definition.
	CDefinition string
}

// Tracepoint holds the specification for a tracepoint sanitizer.
type Tracepoint struct {
	// Group is the tracepoint group. (e.g. "raw_syscalls")
	Group string
	// Name is the tracepoint name. (e.g. "sys_enter")
	Name string
}

// Kprobe holds the specification for a kprobe sanitizer.
type Kprobe struct {
	// Function is the kernel function to attach to. (e.g. "vfs_rmdir")
	Function string
}

// TemplateGoFile returns the templated Go source file for the sanitizer.
func TemplateGoFile(data *SanitizerTemplateData) (string, error) {
	tmpl, err := template.New("sanitizer_go").
		Funcs(funcMap).
		Parse(sanitizerGoTemplate)
	if err != nil {
		return "", fmt.Errorf("parse template: %w", err)
	}

	out := bytes.NewBuffer(nil)
	if err := tmpl.Execute(out, data); err != nil {
		return "", fmt.Errorf("execute template: %w", err)
	}

	formatted, err := format.Source(out.Bytes())
	if err != nil {
		return "", fmt.Errorf("format generated code: %w", err)
	}

	return string(formatted), nil
}

// TemplateGoGenerateFile returns the templated Go generate source file for the sanitizer.
func TemplateGoGenerateFile(data *SanitizerTemplateData) (string, error) {
	tmpl, err := template.New("sanitizer_go_generate").
		Funcs(funcMap).
		Parse(sanitizerGoGenerateTemplate)
	if err != nil {
		return "", fmt.Errorf("parse template: %w", err)
	}

	out := bytes.NewBuffer(nil)
	if err := tmpl.Execute(out, data); err != nil {
		return "", fmt.Errorf("execute template: %w", err)
	}

	formatted, err := format.Source(out.Bytes())
	if err != nil {
		return "", fmt.Errorf("format generated code: %w", err)
	}

	return string(formatted), nil
}

// TemplateBPFCFile returns the templated BPF C source file for the sanitizer.
func TemplateBPFCFile(data *SanitizerTemplateData) (string, error) {
	tmpl, err := template.New("sanitizer_bpf_c").
		Funcs(funcMap).
		Parse(sanitizerBPFCTemplate)
	if err != nil {
		return "", fmt.Errorf("parse template: %w", err)
	}

	out := bytes.NewBuffer(nil)
	if err := tmpl.Execute(out, data); err != nil {
		return "", fmt.Errorf("execute template: %w", err)
	}

	return out.String(), nil
}

var funcMap = template.FuncMap{
	"firstUpper": firstUpper,
	"goFilename": goFilename,
	"cToGoName":  cToGoName,
}

// goFilename converts a C sanitizer name to a Go filename by removing underscores.
func goFilename(s string) string {
	return strings.ReplaceAll(s, "_", "")
}

// firstUpper capitalizes the first letter of the given string.
func firstUpper(s string) string {
	if len(s) == 0 {
		return s
	}
	return strings.ToUpper(s[:1]) + s[1:]
}

// cToGoName converts a C-style name (snake_case) to a Go-style name (camelCase).
func cToGoName(name string) string {
	parts := strings.Split(name, "_")
	for i := range parts {
		if i == 0 {
			continue
		}
		parts[i] = firstUpper(parts[i])
	}
	return strings.Join(parts, "")
}
