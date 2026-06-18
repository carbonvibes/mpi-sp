package canary

//go:generate clang-wrapped c/trigger.c -o bin/trigger.o
//go:generate clang-wrapped c/benign.c -o bin/benign.o
//go:generate clang-wrapped c/trigger_execve.c -o bin/trigger_execve.o
//go:generate clang-wrapped c/benign_execve.c -o bin/benign_execve.o

import (
	_ "embed"
)

//go:embed bin/trigger.o
var TriggerProgram []byte

//go:embed bin/benign.o
var BenignProgram []byte

//go:embed bin/trigger_execve.o
var TriggerExecveProgram []byte

//go:embed bin/benign_execve.o
var BenignExecveProgram []byte
