package attach

import (
	"context"
	"fmt"
	"os"

	bpfruntime "github.com/msanft/SemanticSanitizer/internal/bpf"
	"github.com/msanft/SemanticSanitizer/internal/bpf/canary"
	"github.com/msanft/SemanticSanitizer/internal/bpf/dirownership"
	"github.com/msanft/SemanticSanitizer/internal/bpf/libcfilter"
	"github.com/msanft/SemanticSanitizer/internal/bpf/symlinkmount"
	"github.com/msanft/SemanticSanitizer/internal/bpf/syscallfilter"
	"github.com/msanft/SemanticSanitizer/internal/config"
	"github.com/msanft/SemanticSanitizer/internal/event"
)

// AttachFunc is a function that attaches a BPF program to some place.
type AttachFunc func(conf *config.SanitizerConfig) (*bpfruntime.Attachment, error)

// AttachContext attaches the given attach functions to the system.
// It blocks until the context is done.
// For asynchronous users, it sends a signal on the allRunning channel
// when all functions are attached.
func AttachContext(ctx context.Context, conf *config.SanitizerConfig, allRunning chan struct{}, onEvent func(event.Event)) error {
	attachFuncs := attachFuncsFromConfig(conf)
	if len(attachFuncs) == 0 {
		return fmt.Errorf("no attach functions found for config: %+v", conf)
	}

	attachments := make([]*bpfruntime.Attachment, 0, len(attachFuncs))
	defer func() {
		for i := len(attachments) - 1; i >= 0; i-- {
			_ = attachments[i].Close()
		}
	}()

	for name, fn := range attachFuncs {
		fmt.Printf("Attaching %s...\n", name)
		attachment, err := fn(conf)
		if err != nil {
			return fmt.Errorf("attach function %s: %w", name, err)
		}

		stream, err := event.NewStream(attachment.EventMap, onEvent, func(err error) {
			fmt.Fprintf(os.Stderr, "%s: %v\n", name, err)
		})
		if err != nil {
			_ = attachment.Close()
			return fmt.Errorf("create event stream for %s: %w", name, err)
		}
		attachment.Closers = append(attachment.Closers, stream)
		attachments = append(attachments, attachment)
	}

	// Notify that the attach process is done.
	allRunning <- struct{}{}

	// Wait for the context to be done.
	<-ctx.Done()

	return nil
}

// attachFuncsFromConfig returns the attach functions for the sanitizer config.
func attachFuncsFromConfig(conf *config.SanitizerConfig) map[string]AttachFunc {
	attachFuncs := map[string]AttachFunc{}

	if len(conf.Syscalls) > 0 {
		attachFuncs["syscallfilter"] = syscallfilter.Attach
	}

	if conf.DangerousLibc {
		attachFuncs["libcfilter"] = libcfilter.Attach
	}

	if conf.SymlinkMount {
		attachFuncs["symlinkmount"] = symlinkmount.Attach
	}

	if conf.DirOwnership {
		attachFuncs["dirownership"] = dirownership.Attach
	}

	if len(conf.Canary) > 0 {
		attachFuncs["canary"] = canary.Attach
	}

	return attachFuncs
}
