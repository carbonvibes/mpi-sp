package event

import (
	"errors"
	"fmt"
	"io"
	"sync"

	"github.com/cilium/ebpf"
	"github.com/cilium/ebpf/ringbuf"
)

// Stream consumes sanitizer events from a ring buffer map.
type Stream struct {
	reader *ringbuf.Reader
	wg     sync.WaitGroup
}

// NewStream starts reading events from m and forwards them to onEvent.
func NewStream(m *ebpf.Map, onEvent func(Event), onError func(error)) (*Stream, error) {
	reader, err := ringbuf.NewReader(m)
	if err != nil {
		return nil, fmt.Errorf("create ringbuf reader: %w", err)
	}

	stream := &Stream{reader: reader}
	stream.wg.Add(1)
	go func() {
		defer stream.wg.Done()
		for {
			record, err := reader.Read()
			if err != nil {
				if errors.Is(err, ringbuf.ErrClosed) {
					return
				}
				if onError != nil {
					onError(fmt.Errorf("read ringbuf event: %w", err))
				}
				continue
			}

			evt, err := Parse(record.RawSample)
			if err != nil {
				if onError != nil {
					onError(err)
				}
				continue
			}

			if onEvent != nil {
				onEvent(evt)
			}
		}
	}()

	return stream, nil
}

// Close stops the stream and waits for the reader goroutine to exit.
func (s *Stream) Close() error {
	if s == nil {
		return nil
	}

	err := s.reader.Close()
	s.wg.Wait()
	if errors.Is(err, ringbuf.ErrClosed) {
		return nil
	}
	if err != nil {
		return err
	}
	return nil
}

var _ io.Closer = (*Stream)(nil)
