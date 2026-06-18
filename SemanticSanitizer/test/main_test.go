//go:build integration

package main

import (
	"syscall"
	"testing"

	"github.com/msanft/SemanticSanitizer/internal/config"
	"github.com/msanft/SemanticSanitizer/test/canary"
	"github.com/msanft/SemanticSanitizer/test/dirownership"
	"github.com/msanft/SemanticSanitizer/test/internal/client"
	"github.com/msanft/SemanticSanitizer/test/libcfilter"
	"github.com/msanft/SemanticSanitizer/test/symlinkmount"
	"github.com/msanft/SemanticSanitizer/test/syscallfilter"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"golang.org/x/sys/unix"
)

func TestSyscallFilter(t *testing.T) {
	require := require.New(t)
	assert := assert.New(t)

	conf := &config.SanitizerConfig{
		Syscalls: map[string]config.SyscallConfig{
			"gettimeofday": {
				Number: syscall.SYS_GETTIMEOFDAY,
			},
		},
	}

	c := client.New(conf)

	t.Run("benign", func(t *testing.T) {
		c.Config.Comm = "benign"
		completed, err := c.TestSanitizer(t, syscallfilter.BenignProgram)
		require.NoError(err)
		assert.True(completed)
	})

	t.Run("trigger", func(t *testing.T) {
		c.Config.Comm = "trigger"
		completed, err := c.TestSanitizer(t, syscallfilter.TriggerProgram)
		require.False(completed)
		assert.ErrorContains(err, "signal: killed")
	})
}

func TestLibcGets(t *testing.T) {
	t.Skip("Skipping libc gets test until we can reliably pin libc versions.")

	require := require.New(t)
	assert := assert.New(t)

	conf := &config.SanitizerConfig{
		BinaryPath:    "/nix/store/rmy663w9p7xb202rcln4jjzmvivznmz8-glibc-2.40-66/lib/libc.so.6",
		DangerousLibc: true,
	}

	c := client.New(conf)

	t.Run("benign", func(t *testing.T) {
		c.Config.Comm = "benign"
		completed, err := c.TestSanitizer(t, libcfilter.BenignProgram)
		require.NoError(err)
		assert.True(completed)
	})

	t.Run("trigger", func(t *testing.T) {
		c.Config.Comm = "trigger"
		completed, err := c.TestSanitizer(t, libcfilter.TriggerProgram)
		require.False(completed)
		assert.ErrorContains(err, "signal: killed")
	})
}

func TestSymlinkMount(t *testing.T) {
	require := require.New(t)
	assert := assert.New(t)

	conf := &config.SanitizerConfig{
		SymlinkMount: true,
	}

	c := client.New(conf)

	cleanup := func() {
		unix.Unmount("mnt", unix.MNT_DETACH)
	}

	t.Run("benign", func(t *testing.T) {
		defer cleanup()
		c.Config.Comm = "benign"
		completed, err := c.TestSanitizer(t, symlinkmount.BenignProgram)
		require.NoError(err)
		assert.True(completed)
	})

	t.Run("trigger", func(t *testing.T) {
		defer cleanup()
		c.Config.Comm = "trigger"
		completed, err := c.TestSanitizer(t, symlinkmount.TriggerProgram)
		require.False(completed)
		assert.ErrorContains(err, "signal: killed")
	})
}

func TestDirownership(t *testing.T) {
	require := require.New(t)
	assert := assert.New(t)

	conf := &config.SanitizerConfig{
		DirOwnership: true,
	}

	c := client.New(conf)

	t.Run("benign", func(t *testing.T) {
		c.Config.Comm = "benign"
		completed, err := c.TestSanitizer(t, dirownership.BenignProgram)
		require.NoError(err)
		assert.True(completed)
	})

	t.Run("trigger", func(t *testing.T) {
		c.Config.Comm = "trigger"
		completed, err := c.TestSanitizer(t, dirownership.TriggerProgram)
		require.False(completed)
		assert.ErrorContains(err, "signal: killed")
	})

	// TODO(msanft): Enable once bind-mounting sanitizer for the old
	// mount API is fixed. See dirownership.bpf.c
	// t.Run("trigger2", func(t *testing.T) {
	// 	c.Config.Comm = "trigger2"
	// 	completed, err := c.TestSanitizer(t, dirownership.Trigger2Program)
	// 	require.False(completed)
	// 	assert.ErrorContains(err, "signal: killed")
	// })
}

func TestCanary(t *testing.T) {
	t.Run("direct string arg", func(t *testing.T) {
		conf := &config.SanitizerConfig{
			Canary: map[string]config.CanaryConfig{
				"openat": {
					ArgIndex:  1,
					Substring: "canary",
				},
			},
		}

		c := client.New(conf)

		t.Run("benign", func(t *testing.T) {
			require := require.New(t)
			assert := assert.New(t)

			c.Config.Comm = "benign"
			completed, err := c.TestSanitizer(t, canary.BenignProgram)
			require.NoError(err)
			assert.True(completed)
		})

		t.Run("trigger", func(t *testing.T) {
			require := require.New(t)
			assert := assert.New(t)

			c.Config.Comm = "trigger"
			completed, err := c.TestSanitizer(t, canary.TriggerProgram)
			require.False(completed)
			assert.ErrorContains(err, "signal: killed")
		})
	})

	t.Run("execve argv string array", func(t *testing.T) {
		conf := &config.SanitizerConfig{
			Canary: map[string]config.CanaryConfig{
				"execve": {
					ArgIndex:  1,
					Substring: "pwned",
				},
			},
		}

		c := client.New(conf)

		t.Run("benign", func(t *testing.T) {
			require := require.New(t)
			assert := assert.New(t)

			c.Config.Comm = "trigger"
			completed, err := c.TestSanitizer(t, canary.BenignExecveProgram)
			require.NoError(err)
			assert.True(completed)
		})

		t.Run("trigger", func(t *testing.T) {
			require := require.New(t)
			assert := assert.New(t)

			c.Config.Comm = "trigger"
			completed, err := c.TestSanitizer(t, canary.TriggerExecveProgram)
			require.False(completed)
			assert.ErrorContains(err, "signal: killed")
		})
	})
}
