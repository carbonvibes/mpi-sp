package syscallfilter

import (
	"fmt"
	"io"

	"github.com/cilium/ebpf/link"
	bpfruntime "github.com/msanft/SemanticSanitizer/internal/bpf"
	"github.com/msanft/SemanticSanitizer/internal/config"
)

const syscallDisallowed = uint8(1)

func Attach(conf *config.SanitizerConfig) (*bpfruntime.Attachment, error) {
	objs := syscallfilterObjects{}
	if err := loadSyscallfilterObjects(&objs, nil); err != nil {
		return nil, fmt.Errorf("load syscallfilter objects: %w", err)
	}

	closers := []io.Closer{&objs}
	closeAll := func() {
		for i := len(closers) - 1; i >= 0; i-- {
			_ = closers[i].Close()
		}
	}

	if err := objs.syscallfilterMaps.SemsanConfig.Put(uint32(0), bpfruntime.EncodeComm(conf.Comm)); err != nil {
		closeAll()
		return nil, fmt.Errorf("put config: %w", err)
	}

	for _, sc := range conf.Syscalls {
		if err := objs.syscallfilterMaps.DisallowedSyscalls.Put(uint32(sc.Number), syscallDisallowed); err != nil {
			closeAll()
			return nil, fmt.Errorf("put disallowed syscall: %w", err)
		}
	}

	kp, err := link.Tracepoint("raw_syscalls", "sys_enter", objs.SyscallFilterWrapper, nil)
	if err != nil {
		closeAll()
		return nil, fmt.Errorf("attach to tracepoint: %w", err)
	}
	closers = append(closers, kp)

	return &bpfruntime.Attachment{
		EventMap: objs.syscallfilterMaps.SemsanEvents,
		Closers:  closers,
	}, nil
}
