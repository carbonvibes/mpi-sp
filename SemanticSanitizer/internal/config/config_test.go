package config

import (
	"os"
	"testing"
)

func TestNewFromFileParsesCanaryCamelCaseFields(t *testing.T) {
	path := t.TempDir() + "/config.yaml"
	contents := []byte(`comm: '*'
canary:
  execve:
    argIndex: 1
    substring: pwned
syscalls:
  gettimeofday:
    number: 96
`)

	if err := os.WriteFile(path, contents, 0o644); err != nil {
		t.Fatalf("write config: %v", err)
	}

	conf, err := NewFromFile(path)
	if err != nil {
		t.Fatalf("load config: %v", err)
	}

	rule := conf.Canary["execve"]
	if rule.ArgIndex != 1 {
		t.Fatalf("ArgIndex = %d, want 1", rule.ArgIndex)
	}
	if rule.Substring != "pwned" {
		t.Fatalf("Substring = %q, want %q", rule.Substring, "pwned")
	}

	sc := conf.Syscalls["gettimeofday"]
	if sc.Number != 96 {
		t.Fatalf("Number = %d, want 96", sc.Number)
	}
}
