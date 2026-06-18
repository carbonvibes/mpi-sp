package symlinkmount

import (
	"fmt"
	"io"

	"github.com/cilium/ebpf/link"
	bpfruntime "github.com/msanft/SemanticSanitizer/internal/bpf"
	"github.com/msanft/SemanticSanitizer/internal/config"
)

func Attach(conf *config.SanitizerConfig) (*bpfruntime.Attachment, error) {
	objs := symlinkmountObjects{}
	if err := loadSymlinkmountObjects(&objs, nil); err != nil {
		return nil, fmt.Errorf("load symlinkmount objects: %w", err)
	}

	closers := []io.Closer{&objs}
	closeAll := func() {
		for i := len(closers) - 1; i >= 0; i-- {
			_ = closers[i].Close()
		}
	}

	if err := objs.symlinkmountMaps.SemsanConfig.Put(uint32(0), bpfruntime.EncodeComm(conf.Comm)); err != nil {
		closeAll()
		return nil, fmt.Errorf("put config: %w", err)
	}

	kp, err := link.Kprobe("vfs_symlink", objs.TraceVfsSymlinkWrapper, nil)
	if err != nil {
		closeAll()
		return nil, fmt.Errorf("attach kprobe: %w", err)
	}
	closers = append(closers, kp)

	tp, err := link.Tracepoint("syscalls", "sys_enter_mount", objs.SymlinkMountWrapper, nil)
	if err != nil {
		closeAll()
		return nil, fmt.Errorf("attach to tracepoint: %w", err)
	}
	closers = append(closers, tp)

	return &bpfruntime.Attachment{
		EventMap: objs.symlinkmountMaps.SemsanEvents,
		Closers:  closers,
	}, nil
}
