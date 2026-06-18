package bpf

const (
	// TaskCommLen is linux/sched.h TASK_COMM_LEN, including the trailing NUL.
	TaskCommLen = 16
	// MaxVisibleCommLen is the maximum number of visible bytes in task comm.
	MaxVisibleCommLen = TaskCommLen - 1
)

// EncodeComm encodes a configured comm value the same way the kernel exposes it
// through bpf_get_current_comm(): up to 15 visible bytes plus a trailing NUL.
func EncodeComm(comm string) []byte {
	encoded := make([]byte, TaskCommLen)
	copy(encoded[:MaxVisibleCommLen], []byte(comm))
	return encoded
}
