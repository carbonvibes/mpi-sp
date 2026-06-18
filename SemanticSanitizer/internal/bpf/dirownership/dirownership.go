package dirownership

import (
	"errors"
	"fmt"
	"io"
	"os"

	"github.com/cilium/ebpf/link"
	bpfruntime "github.com/msanft/SemanticSanitizer/internal/bpf"
	"github.com/msanft/SemanticSanitizer/internal/config"
)

func Attach(conf *config.SanitizerConfig) (*bpfruntime.Attachment, error) {
	objs := dirownershipObjects{}
	if err := loadDirownershipObjects(&objs, nil); err != nil {
		return nil, fmt.Errorf("load dirownership objects: %w", err)
	}

	closers := []io.Closer{&objs}
	closeAll := func() {
		for i := len(closers) - 1; i >= 0; i-- {
			_ = closers[i].Close()
		}
	}

	if err := objs.dirownershipMaps.SemsanConfig.Put(uint32(0), bpfruntime.EncodeComm(conf.Comm)); err != nil {
		closeAll()
		return nil, fmt.Errorf("put config: %w", err)
	}

	if conf.DirOwnershipOpenNoFollow {
		fileOpenKprobe, err := link.Kprobe("security_file_open", objs.UnsafeFileOpenWrapper, nil)
		if err != nil {
			closeAll()
			return nil, fmt.Errorf("attach security_file_open kprobe: %w", err)
		}
		closers = append(closers, fileOpenKprobe)

		openTracepoint, err := link.Tracepoint("syscalls", "sys_enter_open", objs.UnsafeOpenWrapper, nil)
		if err != nil && !errors.Is(err, os.ErrNotExist) {
			closeAll()
			return nil, fmt.Errorf("attach sys_enter_open tracepoint: %w", err)
		}
		if openTracepoint != nil {
			closers = append(closers, openTracepoint)
		}

		openatTracepoint, err := link.Tracepoint("syscalls", "sys_enter_openat", objs.UnsafeOpenatWrapper, nil)
		if err != nil {
			closeAll()
			return nil, fmt.Errorf("attach sys_enter_openat tracepoint: %w", err)
		}
		closers = append(closers, openatTracepoint)

		openat2Tracepoint, err := link.Tracepoint("syscalls", "sys_enter_openat2", objs.UnsafeOpenat2Wrapper, nil)
		if err != nil && !errors.Is(err, os.ErrNotExist) {
			closeAll()
			return nil, fmt.Errorf("attach sys_enter_openat2 tracepoint: %w", err)
		}
		if openat2Tracepoint != nil {
			closers = append(closers, openat2Tracepoint)
		}
	}

	moveMountKprobe, err := link.Kprobe("do_move_mount", objs.UnsafeMoveMountWrapper, nil)
	if err != nil && !errors.Is(err, os.ErrNotExist) {
		fmt.Printf("Attaching do_move_mount kprobe failed: %s. "+
			"This function often gets inlined based on compiler versions. "+
			"Therefore, this failure is not treated as a fatal error", err)
	}
	if moveMountKprobe != nil {
		closers = append(closers, moveMountKprobe)
	}

	rmdirKprobe, err := link.Kprobe("vfs_rmdir", objs.UnsafeRmdirWrapper, nil)
	if err != nil {
		closeAll()
		return nil, fmt.Errorf("attach vfs_rmdir kprobe: %w", err)
	}
	closers = append(closers, rmdirKprobe)

	chmodKprobe, err := link.Kprobe("chmod_common", objs.UnsafeChmodWrapper, nil)
	if err != nil {
		closeAll()
		return nil, fmt.Errorf("attach chmod_common kprobe: %w", err)
	}
	closers = append(closers, chmodKprobe)

	chownKprobe, err := link.Kprobe("chown_common", objs.UnsafeChownWrapper, nil)
	if err != nil {
		closeAll()
		return nil, fmt.Errorf("attach chown_common kprobe: %w", err)
	}
	closers = append(closers, chownKprobe)

	bprmExecveKprobe, err := link.Kprobe("bprm_execve", objs.UnsafeExecveWrapper, nil)
	if err != nil {
		closeAll()
		return nil, fmt.Errorf("attach bprm_execve kprobe: %w", err)
	}
	closers = append(closers, bprmExecveKprobe)

	return &bpfruntime.Attachment{
		EventMap: objs.dirownershipMaps.SemsanEvents,
		Closers:  closers,
	}, nil
}
