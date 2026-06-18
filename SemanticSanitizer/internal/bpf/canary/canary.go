package canary

import (
	"fmt"
	"io"

	"github.com/cilium/ebpf/link"
	bpfruntime "github.com/msanft/SemanticSanitizer/internal/bpf"
	"github.com/msanft/SemanticSanitizer/internal/config"
	"golang.org/x/sys/unix"
)

const (
	maxStringLen       = 256
	maxCanaryNeedleLen = 32
)

const (
	canaryMatchDirect uint32 = iota
	canaryMatchStringArray
)

type canaryRule struct {
	ArgIdx        uint32
	MatchMode     uint32
	NeedleLen     uint32
	_             uint32
	DisallowedStr [maxCanaryNeedleLen]byte
}

func Attach(conf *config.SanitizerConfig) (*bpfruntime.Attachment, error) {
	objs := canaryObjects{}
	if err := loadCanaryObjects(&objs, nil); err != nil {
		return nil, fmt.Errorf("load canary objects: %w", err)
	}

	closers := []io.Closer{&objs}
	closeAll := func() {
		for i := len(closers) - 1; i >= 0; i-- {
			_ = closers[i].Close()
		}
	}

	if err := objs.canaryMaps.SemsanConfig.Put(uint32(0), bpfruntime.EncodeComm(conf.Comm)); err != nil {
		closeAll()
		return nil, fmt.Errorf("put config: %w", err)
	}

	rules := make(map[uint32]canaryRule)
	for sc, scConf := range conf.Canary {
		syscallNum, err := getSyscallNumber(sc)
		if err != nil {
			return nil, fmt.Errorf("get syscall number for %s: %w", sc, err)
		}
		if scConf.ArgIndex < 0 || scConf.ArgIndex > 5 {
			return nil, fmt.Errorf("invalid arg index %d for %s", scConf.ArgIndex, sc)
		}
		if len(scConf.Substring) == 0 {
			return nil, fmt.Errorf("substring for %s must not be empty", sc)
		}
		if len(scConf.Substring) > maxCanaryNeedleLen {
			return nil, fmt.Errorf("substring for %s must be at most %d bytes", sc, maxCanaryNeedleLen)
		}

		var disallowedStr [maxCanaryNeedleLen]byte
		copy(disallowedStr[:], scConf.Substring)

		rule := canaryRule{
			ArgIdx:        uint32(scConf.ArgIndex),
			MatchMode:     matchModeForRule(sc, scConf.ArgIndex),
			NeedleLen:     uint32(len(scConf.Substring)),
			DisallowedStr: disallowedStr,
		}

		rules[uint32(syscallNum)] = rule
	}

	for syscallNum, rule := range rules {
		if err := objs.canaryMaps.Canaries.Put(syscallNum, rule); err != nil {
			closeAll()
			return nil, fmt.Errorf("put canary rules for syscall %d: %w", syscallNum, err)
		}
	}

	kp, err := link.Tracepoint("raw_syscalls", "sys_enter", objs.CanaryFilterWrapper, nil)
	if err != nil {
		closeAll()
		return nil, fmt.Errorf("attach to tracepoint: %w", err)
	}
	closers = append(closers, kp)

	return &bpfruntime.Attachment{
		EventMap: objs.canaryMaps.SemsanEvents,
		Closers:  closers,
	}, nil
}

func matchModeForRule(syscallName string, argIndex int) uint32 {
	switch syscallName {
	case "execve":
		if argIndex == 1 || argIndex == 2 {
			return canaryMatchStringArray
		}
	case "execveat":
		if argIndex == 2 || argIndex == 3 {
			return canaryMatchStringArray
		}
	}

	return canaryMatchDirect
}

func getSyscallNumber(name string) (int, error) {
	syscallMap := map[string]int{
		"read":     unix.SYS_READ,
		"write":    unix.SYS_WRITE,
		"open":     unix.SYS_OPEN,
		"openat":   unix.SYS_OPENAT,
		"execve":   unix.SYS_EXECVE,
		"execveat": unix.SYS_EXECVEAT,
	}

	num, ok := syscallMap[name]
	if !ok {
		return 0, fmt.Errorf("unknown syscall: %s", name)
	}
	return num, nil
}
