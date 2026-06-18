package event

import (
	"bytes"
	"encoding/binary"
	"testing"
)

func TestParseAndFormat(t *testing.T) {
	raw := rawEvent{
		Pid:       1234,
		Tgid:      1234,
		SyscallID: 59,
		ArgIdx:    1,
		Action:    1,
	}
	copy(raw.Comm[:], []byte("trigger"))
	copy(raw.Sanitizer[:], []byte("canary"))
	copy(raw.Operation[:], []byte("syscall_arg_substring"))
	copy(raw.Subject[:], []byte("pwned"))

	buf := new(bytes.Buffer)
	if err := binary.Write(buf, binary.LittleEndian, raw); err != nil {
		t.Fatalf("write raw event: %v", err)
	}

	evt, err := Parse(buf.Bytes())
	if err != nil {
		t.Fatalf("parse event: %v", err)
	}

	got := evt.String()
	want := `[trigger:1234] Canary triggered: detected disallowed substring "pwned" in arg 1 of syscall execve`
	if got != want {
		t.Fatalf("unexpected formatted event:\n got: %s\nwant: %s", got, want)
	}
}
