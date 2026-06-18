package testsanitizer

import (
	"fmt"

	"github.com/cilium/ebpf/link"
	"github.com/msanft/SemanticSanitizer/internal/config"
)

type TestStruct struct {
	Field1 string
	Field2 int
}

func Attach(conf *config.SanitizerConfig) ([]link.Link, error) {
	objs := testsanitizerObjects{}
	if err := loadTestsanitizerObjects(&objs, nil); err != nil {
		return nil, fmt.Errorf("load testsanitizer objects: %w", err)
	}
	defer objs.Close()

	// (Optional) TODO: Use configuration values and transform to BPF map, as necessary.

	sysEnterTracepoint, err := link.Tracepoint(
		"raw_syscalls", "sys_enter", objs.SysEnterWrapper, nil,
	)
	if err != nil {
		return nil, fmt.Errorf("attach to tracepoint 'sysEnter': %w", err)
	}

	return []link.Link{
		sysEnterTracepoint,
	}, nil
}
