package canary

//go:generate go run github.com/cilium/ebpf/cmd/bpf2go -tags linux canary canary.bpf.c
