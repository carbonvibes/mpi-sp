package libcfilter

import (
	"fmt"
	"io"

	"github.com/cilium/ebpf/link"
	bpfruntime "github.com/msanft/SemanticSanitizer/internal/bpf"
	"github.com/msanft/SemanticSanitizer/internal/config"
)

func Attach(conf *config.SanitizerConfig) (*bpfruntime.Attachment, error) {
	objs := libcfilterObjects{}
	if err := loadLibcfilterObjects(&objs, nil); err != nil {
		return nil, fmt.Errorf("load libcfilter objects: %w", err)
	}

	closers := []io.Closer{&objs}
	closeAll := func() {
		for i := len(closers) - 1; i >= 0; i-- {
			_ = closers[i].Close()
		}
	}

	if err := objs.libcfilterMaps.SemsanConfig.Put(uint32(0), bpfruntime.EncodeComm(conf.Comm)); err != nil {
		closeAll()
		return nil, fmt.Errorf("put config: %w", err)
	}

	ex, err := link.OpenExecutable(conf.BinaryPath)
	if err != nil {
		closeAll()
		return nil, fmt.Errorf("open executable: %w", err)
	}

	up, err := ex.Uprobe("__gets_chk", objs.LibcFilterWrapper, &link.UprobeOptions{})
	if err != nil {
		closeAll()
		return nil, fmt.Errorf("attach uprobe: %w", err)
	}
	closers = append(closers, up)

	return &bpfruntime.Attachment{
		EventMap: objs.libcfilterMaps.SemsanEvents,
		Closers:  closers,
	}, nil
}
