package cmd

import (
	"fmt"

	"github.com/msanft/SemanticSanitizer/bootstrap/internal/bootstrap"
	"github.com/msanft/SemanticSanitizer/bootstrap/internal/spec"
	"github.com/spf13/cobra"
)

func NewSanitizerCmd() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "sanitizer",
		Short: "bootstrap a new sanitizer within the SemanticSanitizer codebase",
		Long:  "TODO",
		RunE:  runSanitizer,
	}

	cmd.Flags().StringP("spec", "s", spec.DefaultPath, "path to the sanitizer spec file")
	cmd.MarkFlagFilename("spec")
	cmd.MarkFlagRequired("spec")

	return cmd
}

func runSanitizer(cmd *cobra.Command, args []string) error {
	specPath, err := cmd.Flags().GetString("spec")
	if err != nil {
		return fmt.Errorf("get spec flag: %w", err)
	}

	sanitizerSpec, err := spec.NewFromFile(specPath)
	if err != nil {
		return fmt.Errorf("load sanitizer spec: %w", err)
	}

	sanitizerPath, err := bootstrap.NewSanitizer(sanitizerSpec)
	if err != nil {
		return fmt.Errorf("bootstrap sanitizer: %w", err)
	}

	cmd.Printf("The sanitizer has successfully been generated in %s\n", sanitizerPath)
	cmd.Println("Remember to run 'just codegen' or 'go generate ./...' to generate the eBPF-Go bindings.")

	return nil
}
