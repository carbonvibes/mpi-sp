package trigger

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
)

// Run executes the trigger program in a separate process.
// It is meant to be run in a separate goroutine, and reports errors
// through the errChan channel.
// A message is sent to the completed channel when the program is done.
func Run(ctx context.Context, program []byte, errChan chan error, completed chan struct{}) {
	tmpDir, err := os.MkdirTemp("", "trigger")
	if err != nil {
		errChan <- fmt.Errorf("create tempdir: %w", err)
		return
	}
	if err := os.Chdir(tmpDir); err != nil {
		errChan <- fmt.Errorf("change working directory: %w", err)
		return
	}

	defer func() {
		if err := os.RemoveAll(tmpDir); err != nil {
			errChan <- fmt.Errorf("remove tempdir: %w. clean up the tempdir manually", err)
		}
	}()

	triggerFile := filepath.Join(tmpDir, "trigger")
	if err := os.WriteFile(triggerFile, program, 0o755); err != nil {
		errChan <- fmt.Errorf("write trigger binary: %w", err)
		return
	}

	cmd := exec.CommandContext(ctx, triggerFile)
	cmd.Stderr = os.Stderr
	cmd.Stdout = os.Stdout

	if err := cmd.Run(); err != nil {
		errChan <- fmt.Errorf("run trigger binary: %w", err)
	}
	completed <- struct{}{}
}
