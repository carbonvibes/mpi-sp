package cmd

import (
	"fmt"

	"github.com/msanft/SemanticSanitizer/internal/attach"
	"github.com/msanft/SemanticSanitizer/internal/config"
	"github.com/msanft/SemanticSanitizer/internal/event"
	"github.com/spf13/cobra"
)

func NewAttachCmd() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "attach",
		Short: "attach the sanitizer",
		Long:  "TODO",
		RunE:  runAttach,
	}

	cmd.Flags().StringP("config", "c", config.DefaultPath, "path to the configuration file")
	cmd.MarkFlagFilename("config")

	return cmd
}

func runAttach(cmd *cobra.Command, args []string) error {
	configPath, err := cmd.Flags().GetString("config")
	if err != nil {
		return fmt.Errorf("get config flag: %w", err)
	}

	conf, err := config.NewFromFile(configPath)
	if err != nil {
		return fmt.Errorf("read config file: %w", err)
	}

	allRunning := make(chan struct{})
	go func() {
		<-allRunning
		fmt.Println("All sanitizers are running")
	}()

	if err = attach.AttachContext(cmd.Context(), conf, allRunning, func(evt event.Event) {
		fmt.Println(evt.String())
	}); err != nil {
		return fmt.Errorf("attach sanitizer: %w", err)
	}

	return nil
}
