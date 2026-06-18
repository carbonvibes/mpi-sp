package cmd

import (
	"fmt"

	"github.com/msanft/SemanticSanitizer/internal/config"
	"github.com/spf13/cobra"
)

func NewConfigCmd() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "config",
		Short: "handle configuration",
		Long:  "TODO",
	}

	cmd.AddCommand(
		newConfigGenerateCmd(),
	)

	return cmd
}

func newConfigGenerateCmd() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "generate",
		Short: "generate a new configuration file",
		Long:  "TODO",
		RunE:  runConfigGenerate,
	}

	cmd.Flags().StringP("output", "o", config.DefaultPath, "output path")
	cmd.MarkFlagFilename("output")

	return cmd
}

func runConfigGenerate(cmd *cobra.Command, args []string) error {
	if err := config.NewSanitizerConfig().WriteToFile(config.DefaultPath); err != nil {
		return fmt.Errorf("write config file: %w", err)
	}

	return nil
}
