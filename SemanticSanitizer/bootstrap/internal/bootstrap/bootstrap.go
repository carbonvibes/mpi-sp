package bootstrap

import (
	"fmt"
	"os"
	"strings"

	"github.com/msanft/SemanticSanitizer/bootstrap/internal/spec"
	"github.com/msanft/SemanticSanitizer/bootstrap/internal/structparser"
	"github.com/msanft/SemanticSanitizer/bootstrap/internal/template"
)

// NewSanitizer bootstraps a new sanitizer based on the given spec.
// It returns the path to the created sanitizer.
func NewSanitizer(spec *spec.SanitizerSpec) (string, error) {
	if err := validateTree(spec.Name); err != nil {
		return "", fmt.Errorf("validate tree: %w", err)
	}

	data := &template.SanitizerTemplateData{
		Name:           spec.Name,
		BindingStructs: []template.BindingStruct{},
		Maps:           []template.BPFMap{},
		Tracepoints:    make(map[string]template.Tracepoint),
		Kprobes:        make(map[string]template.Kprobe),
	}

	for name, tracepoint := range spec.Tracepoints {
		data.Tracepoints[name] = template.Tracepoint{
			Name:  tracepoint.Name,
			Group: tracepoint.Group,
		}
	}

	for name, kprobe := range spec.Kprobes {
		data.Kprobes[name] = template.Kprobe{
			Function: kprobe.Function,
		}
	}

	for _, m := range spec.Maps {
		bpfType := "BPF_MAP_TYPE_HASH"
		switch m.Type {
		case "array":
			bpfType = "BPF_MAP_TYPE_ARRAY"
		case "lru_hashmap":
			bpfType = "BPF_MAP_TYPE_LRU_HASH"
		}
		data.Maps = append(data.Maps, template.BPFMap{
			Name:       m.Name,
			Type:       bpfType,
			KeyType:    m.KeyType,
			ValueType:  m.ValueType,
			MaxEntries: m.MaxEntries,
		})
	}

	for _, bindingStruct := range spec.BindingStructs {
		cDefinition, err := structparser.ToCStruct(bindingStruct.Definition)
		if err != nil {
			return "", fmt.Errorf("convert binding struct %q to C: %w", bindingStruct.Definition, err)
		}
		templateBindingStruct := template.BindingStruct{
			GoDefinition: bindingStruct.Definition,
			CDefinition:  cDefinition,
		}
		data.BindingStructs = append(data.BindingStructs, templateBindingStruct)
	}

	outPath, err := createFiles(data)
	if err != nil {
		return "", fmt.Errorf("create sanitizer files: %w", err)
	}

	return outPath, nil
}

// createFiles creates the necessary directories files for the sanitizer.
func createFiles(data *template.SanitizerTemplateData) (string, error) {
	goName := strings.ReplaceAll(data.Name, "_", "")

	outPath := fmt.Sprintf("internal/bpf/%s", goName)
	if err := os.MkdirAll(outPath, 0o755); err != nil {
		return "", fmt.Errorf("create sanitizer dir: %w", err)
	}

	goFileContent, err := template.TemplateGoFile(data)
	if err != nil {
		return "", fmt.Errorf("template Go file: %w", err)
	}

	if err := os.WriteFile(
		fmt.Sprintf("internal/bpf/%s/%s.go", goName, goName),
		[]byte(goFileContent),
		0o644,
	); err != nil {
		return "", fmt.Errorf("write Go file: %w", err)
	}

	goGenerateFileContent, err := template.TemplateGoGenerateFile(data)
	if err != nil {
		return "", fmt.Errorf("template Go generate file: %w", err)
	}

	if err := os.WriteFile(
		fmt.Sprintf("internal/bpf/%s/generate.go", goName),
		[]byte(goGenerateFileContent),
		0o644,
	); err != nil {
		return "", fmt.Errorf("write Go file: %w", err)
	}

	bpfCFileContent, err := template.TemplateBPFCFile(data)
	if err != nil {
		return "", fmt.Errorf("template BPF C file: %w", err)
	}

	if err := os.WriteFile(
		fmt.Sprintf("internal/bpf/%s/%s.bpf.c", goName, goName),
		[]byte(bpfCFileContent),
		0o644,
	); err != nil {
		return "", fmt.Errorf("write BPF C file: %w", err)
	}

	return outPath, nil
}

// validateTree checks whether the command is executed within
// the SemanticSanitizer codebase, and whether a sanitizer
// with the given name already exists.
func validateTree(name string) error {
	if _, err := os.Stat("internal/bpf"); err != nil {
		return fmt.Errorf("internal/bpf could not be queried: %w", err)
	}

	existingSanitizers, err := os.ReadDir("internal/bpf")
	if err != nil {
		return fmt.Errorf("get existing sanitizers: %w", err)
	}

	for _, sanitizer := range existingSanitizers {
		if sanitizer.Name() == name {
			return fmt.Errorf("sanitizer %q already exists", sanitizer.Name())
		}
	}

	return nil
}
