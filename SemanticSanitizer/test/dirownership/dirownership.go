package dirownership

//go:generate clang-wrapped c/trigger.c -o bin/trigger.o
//go:generate clang-wrapped c/trigger2.c -o bin/trigger2.o
//go:generate clang-wrapped c/benign.c -o bin/benign.o

import (
	_ "embed"
)

//go:embed bin/trigger.o
var TriggerProgram []byte

//go:embed bin/trigger2.o
var Trigger2Program []byte

//go:embed bin/benign.o
var BenignProgram []byte
