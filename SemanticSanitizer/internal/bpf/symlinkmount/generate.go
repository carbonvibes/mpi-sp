package symlinkmount

//go:generate go run github.com/cilium/ebpf/cmd/bpf2go -tags linux symlinkmount symlinkmount.bpf.c
