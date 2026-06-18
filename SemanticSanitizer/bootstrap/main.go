package main

import (
	"context"
	"fmt"
	"os"

	"github.com/msanft/SemanticSanitizer/bootstrap/cmd"
	"github.com/msanft/SemanticSanitizer/internal/process"
	"github.com/spf13/cobra"
)

func main() {
	if err := execute(); err != nil {
		os.Exit(1)
	}
}

func execute() error {
	cmd, err := newRootCmd()
	if err != nil {
		return fmt.Errorf("create root cmd: %w", err)
	}

	ctx, cancel := process.SignalContext(context.Background(), os.Interrupt)
	defer cancel()
	return cmd.ExecuteContext(ctx)
}

func newRootCmd() (*cobra.Command, error) {
	root := &cobra.Command{
		Use:     "bootstrap",
		Short:   "bootstrap",
		Version: "0.0.1",
	}
	root.SetOut(os.Stdout)

	root.InitDefaultVersionFlag()
	root.AddCommand(
		cmd.NewSanitizerCmd(),
		cmd.NewSpecCmd(),
	)

	return root, nil
}
