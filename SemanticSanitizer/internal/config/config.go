package config

import (
	"fmt"
	"os"

	"gopkg.in/yaml.v3"
)

// DefaultPath is the default path for the configuration file.
const DefaultPath = "config.yaml"

// SanitizerConfig holds the configuration for SemanticSanitizer.
type SanitizerConfig struct {
	// Comm is the task comm to sanitize for. Linux task comm is limited to 15
	// visible bytes plus a trailing NUL, so longer values are truncated.
	Comm string `yaml:"comm"`
	// BinaryPath is the file path of the executable to search for symbols.
	// Only required for uprobes.
	BinaryPath string `yaml:"binaryPath"`
	// Syscalls holds the configuration for syscall sanitization.
	Syscalls map[string]SyscallConfig `yaml:"syscalls"`
	// DangerousLibc enables sanitization of dangerous libc functions.
	DangerousLibc bool `yaml:"dangerousLibc"`
	// SymlinkMount enables sanitization of symlink mounts.
	SymlinkMount bool `yaml:"symlinkMount"`
	// DirOwnership enables sanitization of unsafe actions on directories.
	DirOwnership bool `yaml:"dirOwnership"`
	// DirOwnershipOpenNoFollow enables the higher-risk open/openat/openat2
	// dirownership checks for writable opens without O_NOFOLLOW.
	DirOwnershipOpenNoFollow bool `yaml:"dirOwnershipOpenNoFollow"`
	// Canary enables sanitization based on syscall arguments.
	Canary map[string]CanaryConfig `yaml:"canary"`
}

// Syscalls holds the configuration for syscall sanitization.
type SyscallConfig struct {
	// Number specifies the syscall number.
	Number int `yaml:"number"`
}

// CanaryConfig holds the configuration for a canary sanitization rule.
type CanaryConfig struct {
	// ArgIndex specifies the index of the syscall argument to check.
	ArgIndex int `yaml:"argIndex"`
	// Substring specifies the disallowed substring to check for.
	Substring string `yaml:"substring"`
}

// NewSanitizerConfig returns a new SanitizerConfig.
func NewSanitizerConfig() *SanitizerConfig {
	return &SanitizerConfig{
		Comm: "*",
	}
}

// NewFromFile returns a new SanitizerConfig from the file at the given path.
func NewFromFile(path string) (*SanitizerConfig, error) {
	f, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("read config file: %w", err)
	}

	config := &SanitizerConfig{}
	if err = yaml.Unmarshal(f, config); err != nil {
		return nil, fmt.Errorf("unmarshal config: %w", err)
	}

	return config, nil
}

// String returns the string representation of the SanitizerConfig.
func (c *SanitizerConfig) WriteToFile(path string) error {
	b, err := yaml.Marshal(c)
	if err != nil {
		return fmt.Errorf("marshal config: %w", err)
	}

	if err := os.WriteFile(path, []byte(b), 0o644); err != nil {
		return fmt.Errorf("write config file: %w", err)
	}
	return nil
}
