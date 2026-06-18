package syscallfilter

//go:generate go run github.com/cilium/ebpf/cmd/bpf2go -tags linux syscallfilter syscallfilter.bpf.c
