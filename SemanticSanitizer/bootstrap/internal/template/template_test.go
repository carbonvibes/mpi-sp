package template

import (
	"testing"

	_ "embed"

	"github.com/stretchr/testify/require"
)

var (
	//go:embed testdata/expected.go
	expectedSanitizerGoSource string
	//go:embed testdata/expected_generate.go
	expectedSanitizerGoGenerateSource string
	//go:embed testdata/expected.bpf.c
	expectedSanitizerBPFCSource string
)

var testSanitizerSpec = &SanitizerTemplateData{
	Name: "test_sanitizer",
	BindingStructs: []BindingStruct{
		{
			GoDefinition: "type TestStruct struct { Field1 string; Field2 int }",
			CDefinition:  "struct test_struct { char field1[64]; int field2; };",
		},
	},
	Tracepoints: map[string]Tracepoint{
		"sys_enter": {
			Group: "raw_syscalls",
			Name:  "sys_enter",
		},
	},
}

func TestTemplateGoFile(t *testing.T) {
	require := require.New(t)

	out, err := TemplateGoFile(testSanitizerSpec)
	require.NoError(err)
	require.Equal(expectedSanitizerGoSource, out)
}

func TestTemplateGoGenerateFile(t *testing.T) {
	require := require.New(t)

	out, err := TemplateGoGenerateFile(testSanitizerSpec)
	require.NoError(err)
	require.Equal(expectedSanitizerGoGenerateSource, out)
}

func TestTemplateBPFCFile(t *testing.T) {
	require := require.New(t)

	out, err := TemplateBPFCFile(testSanitizerSpec)
	require.NoError(err)
	require.Equal(expectedSanitizerBPFCSource, out)
}

func TestCToGoName(t *testing.T) {
	require := require.New(t)

	testCases := map[string]string{
		"simple":                 "simple",
		"with_underscore":        "withUnderscore",
		"multiple_parts_example": "multiplePartsExample",
		"alreadyCamelCase":       "alreadyCamelCase",
		"":                       "",
	}

	for input, expected := range testCases {
		require.Equal(expected, cToGoName(input))
	}
}
