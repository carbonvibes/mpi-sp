package spec

import (
	"fmt"
	"os"
	"regexp"

	"gopkg.in/yaml.v3"
)

// DefaultPath is the default path for the specification file.
const DefaultPath = "spec.yaml"

// nameRegex is the validation regex for (C) sanitizer names.
var nameRegex = regexp.MustCompile(`^[a-z][a-z_]+$`)

// BPFMapType is the type of BPF map.
type BPFMapType string

const (
	BPFMapTypeHashmap    BPFMapType = "hashmap"
	BPFMapTypeArray      BPFMapType = "array"
	BPFMapTypeLRUHashmap BPFMapType = "lru_hashmap"
)

// SanitizerSpec holds the specification for a sanitizer.
type SanitizerSpec struct {
	// Name is the name of the sanitizer.
	Name string `yaml:"name"`
	// BindingStructs is the list of binding structs to generate.
	BindingStructs []BindingStruct `yaml:"bindingStructs"`
	// Maps is the list of BPF maps to generate.
	Maps []BPFMapSpec `yaml:"maps"`
	// Tracepoints is a map of names to tracepoint sanitizers to generate.
	Tracepoints map[string]TracepointSpec `yaml:"tracepoints"`
	// Kprobes is a map of names to kprobe sanitizers to generate.
	Kprobes map[string]KprobeSpec `yaml:"kprobes"`
}

// BPFMapSpec holds the specification for a BPF map.
type BPFMapSpec struct {
	// Name is the name of the map.
	Name string `yaml:"name"`
	// Type is the type of the map (hashmap or array).
	Type BPFMapType `yaml:"type"`
	// KeyType is the C type of the map key.
	KeyType string `yaml:"keyType"`
	// ValueType is the C type of the map value.
	ValueType string `yaml:"valueType"`
	// MaxEntries is the maximum number of entries in the map.
	MaxEntries int `yaml:"maxEntries"`
}

// BindingStruct holds the definition of a binding struct.
type BindingStruct struct {
	// Definition is the Go struct definition.
	Definition string
}

// TracepointSpec holds the specification for a tracepoint sanitizer.
type TracepointSpec struct {
	// Group is the tracepoint group. (e.g. "raw_syscalls")
	Group string `yaml:"group"`
	// Name is the tracepoint name. (e.g. "sys_enter")
	Name string `yaml:"name"`
}

// KprobeSpec holds the specification for a kprobe sanitizer.
type KprobeSpec struct {
	// Function is the kernel function to attach the kprobe to. (e.g. "vfs_rmdir")
	Function string `yaml:"function"`
}

// New returns a new [SanitizerSpec].
func New() *SanitizerSpec {
	return &SanitizerSpec{
		BindingStructs: []BindingStruct{},
		Maps:           []BPFMapSpec{},
		Tracepoints:    make(map[string]TracepointSpec),
		Kprobes:        make(map[string]KprobeSpec),
	}
}

// Example returns a sample [SanitizerSpec].
func Example() *SanitizerSpec {
	return &SanitizerSpec{
		Name: "example_sanitizer",
		BindingStructs: []BindingStruct{
			{Definition: "type ExampleStruct struct { Field1 string; Field2 uint32 }"},
		},
		Maps: []BPFMapSpec{
			{
				Name:       "events",
				Type:       BPFMapTypeHashmap,
				KeyType:    "uint64",
				ValueType:  "syscall_event_t",
				MaxEntries: 1024,
			},
		},
		Tracepoints: map[string]TracepointSpec{
			"sys_enter": {
				Group: "raw_syscalls",
				Name:  "sys_enter",
			},
		},
		Kprobes: map[string]KprobeSpec{
			"vfs_rmdir": {
				Function: "vfs_rmdir",
			},
		},
	}
}

// NewFromFile returns a new [SanitizerSpec] from the file at the given path.
func NewFromFile(path string) (*SanitizerSpec, error) {
	f, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("read spec file: %w", err)
	}

	spec := &SanitizerSpec{}
	if err = yaml.Unmarshal(f, spec); err != nil {
		return nil, fmt.Errorf("unmarshal spec: %w", err)
	}

	return spec, nil
}

// String returns the string representation of the [SanitizerSpec].
func (s *SanitizerSpec) WriteToFile(path string) error {
	b, err := yaml.Marshal(s)
	if err != nil {
		return fmt.Errorf("marshal spec: %w", err)
	}

	if err := os.WriteFile(path, []byte(b), 0o644); err != nil {
		return fmt.Errorf("write spec file: %w", err)
	}
	return nil
}

// Validate validates the [SanitizerSpec].
func (s *SanitizerSpec) Validate() error {
	if !nameRegex.MatchString(s.Name) {
		return fmt.Errorf("invalid sanitizer name %q: must match regex %q", s.Name, nameRegex.String())
	}

	for _, structDef := range s.BindingStructs {
		if structDef.Definition == "" {
			return fmt.Errorf("binding struct %q has empty definition", structDef.Definition)
		}
	}

	for _, mapSpec := range s.Maps {
		if mapSpec.Name == "" {
			return fmt.Errorf("map has empty name")
		}
		if mapSpec.Type != BPFMapTypeHashmap && mapSpec.Type != BPFMapTypeArray && mapSpec.Type != BPFMapTypeLRUHashmap {
			return fmt.Errorf("map %q has invalid type %q: must be 'hashmap', 'array', or 'lru_hashmap'", mapSpec.Name, mapSpec.Type)
		}
		if mapSpec.KeyType == "" {
			return fmt.Errorf("map %q has empty keyType", mapSpec.Name)
		}
		if mapSpec.ValueType == "" {
			return fmt.Errorf("map %q has empty valueType", mapSpec.Name)
		}
		if mapSpec.MaxEntries <= 0 {
			return fmt.Errorf("map %q has invalid maxEntries: must be > 0", mapSpec.Name)
		}
	}

	for tpName, tpSpec := range s.Tracepoints {
		if !nameRegex.MatchString(tpName) {
			return fmt.Errorf("invalid tracepoint sanitizer name %q: must match regex %q", tpName, nameRegex.String())
		}
		if tpSpec.Group == "" {
			return fmt.Errorf("tracepoint %q has empty group", tpName)
		}
		if tpSpec.Name == "" {
			return fmt.Errorf("tracepoint %q has empty name", tpName)
		}
	}

	for kpName, kpSpec := range s.Kprobes {
		if !nameRegex.MatchString(kpName) {
			return fmt.Errorf("invalid kprobe sanitizer name %q: must match regex %q", kpName, nameRegex.String())
		}
		if kpSpec.Function == "" {
			return fmt.Errorf("kprobe %q has empty function", kpName)
		}
	}

	return nil
}
