package event

import (
	"bytes"
	"encoding/binary"
	"fmt"
	"strconv"
	"strings"

	"golang.org/x/sys/unix"
)

const (
	taskCommLen        = 16
	eventNameLen       = 32
	eventTextLen       = 128
	eventActionFinding = 1
)

type rawEvent struct {
	TsNs      uint64
	Pid       uint32
	Tgid      uint32
	SyscallID int32
	ArgIdx    int32
	Action    uint32
	Comm      [taskCommLen]byte
	Sanitizer [eventNameLen]byte
	Operation [eventNameLen]byte
	Subject   [eventTextLen]byte
	Object    [eventTextLen]byte
}

// Event is a user-space representation of a sanitizer event.
type Event struct {
	TsNs      uint64
	Pid       uint32
	Tgid      uint32
	SyscallID int32
	ArgIdx    int32
	Action    uint32
	Comm      string
	Sanitizer string
	Operation string
	Subject   string
	Object    string
}

// Parse decodes a raw ring buffer sample into an Event.
func Parse(sample []byte) (Event, error) {
	var raw rawEvent
	if err := binary.Read(bytes.NewReader(sample), binary.LittleEndian, &raw); err != nil {
		return Event{}, fmt.Errorf("decode event: %w", err)
	}

	return Event{
		TsNs:      raw.TsNs,
		Pid:       raw.Pid,
		Tgid:      raw.Tgid,
		SyscallID: raw.SyscallID,
		ArgIdx:    raw.ArgIdx,
		Action:    raw.Action,
		Comm:      unix.ByteSliceToString(raw.Comm[:]),
		Sanitizer: unix.ByteSliceToString(raw.Sanitizer[:]),
		Operation: unix.ByteSliceToString(raw.Operation[:]),
		Subject:   unix.ByteSliceToString(raw.Subject[:]),
		Object:    unix.ByteSliceToString(raw.Object[:]),
	}, nil
}

// String formats the event for CLI output.
func (e Event) String() string {
	msg := e.message()
	if e.Comm == "" {
		return msg
	}
	return fmt.Sprintf("[%s:%d] %s", e.Comm, e.Tgid, msg)
}

func (e Event) message() string {
	switch e.Sanitizer {
	case "dirownership":
		switch e.Operation {
		case "move_mount":
			return fmt.Sprintf("Unsafe mount move by root user: %s", e.Subject)
		case "chmod":
			return fmt.Sprintf("Unsafe chmod by root user: %s", e.Subject)
		case "chown":
			return fmt.Sprintf("Unsafe chown by root user: %s", e.Subject)
		case "rmdir":
			return fmt.Sprintf("Unsafe rmdir by root user: %s", e.Subject)
		case "execve":
			return fmt.Sprintf("Unsafe execve by root user: %s", e.Subject)
		}
	case "symlinkmount":
		return fmt.Sprintf("Symlink mount detected: %s -> %s", e.Subject, e.Object)
	case "canary":
		return fmt.Sprintf("Canary triggered: detected disallowed substring %q in arg %d of syscall %s", e.Subject, e.ArgIdx, syscallName(e.SyscallID))
	case "syscallfilter":
		return fmt.Sprintf("Disallowed syscall: %s", syscallName(e.SyscallID))
	case "libcfilter":
		if e.Subject != "" {
			return fmt.Sprintf("Dangerous libc function call: %s", e.Subject)
		}
		return "Dangerous libc function call"
	}

	parts := []string{strings.TrimSpace(e.Sanitizer), strings.TrimSpace(e.Operation), strings.TrimSpace(e.Subject), strings.TrimSpace(e.Object)}
	filtered := parts[:0]
	for _, part := range parts {
		if part != "" {
			filtered = append(filtered, part)
		}
	}
	if len(filtered) == 0 {
		return "sanitizer event"
	}
	return strings.Join(filtered, " ")
}

func syscallName(id int32) string {
	switch id {
	case unix.SYS_READ:
		return "read"
	case unix.SYS_WRITE:
		return "write"
	case unix.SYS_OPEN:
		return "open"
	case unix.SYS_OPENAT:
		return "openat"
	case unix.SYS_EXECVE:
		return "execve"
	case unix.SYS_EXECVEAT:
		return "execveat"
	default:
		return "syscall#" + strconv.FormatInt(int64(id), 10)
	}
}
