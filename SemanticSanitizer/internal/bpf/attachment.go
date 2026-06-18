package bpf

import (
	"io"

	"github.com/cilium/ebpf"
)

// Attachment holds the resources of an attached sanitizer.
type Attachment struct {
	EventMap *ebpf.Map
	Closers  []io.Closer
}

// Close releases all resources associated with the attachment.
func (a *Attachment) Close() error {
	var firstErr error
	for i := len(a.Closers) - 1; i >= 0; i-- {
		if err := a.Closers[i].Close(); err != nil && firstErr == nil {
			firstErr = err
		}
	}
	return firstErr
}
