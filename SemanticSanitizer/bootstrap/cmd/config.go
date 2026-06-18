package cmd

import (
	"fmt"

	"github.com/msanft/SemanticSanitizer/bootstrap/internal/spec"
	"github.com/spf13/cobra"
)

func NewSpecCmd() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "spec",
		Short: "create a new specification file for a sanitizer",
		Long:  "TODO",
		RunE:  runConfigGenerate,
	}

	cmd.Flags().StringP("output", "o", spec.DefaultPath, "output path")
	cmd.MarkFlagFilename("output")

	return cmd
}

func runConfigGenerate(cmd *cobra.Command, args []string) error {
	if err := spec.Example().WriteToFile(spec.DefaultPath); err != nil {
		return fmt.Errorf("write spec file: %w", err)
	}

	return nil
}
