package client

import (
	"context"
	"fmt"
	"testing"

	"github.com/msanft/SemanticSanitizer/internal/attach"
	"github.com/msanft/SemanticSanitizer/internal/config"
	"github.com/msanft/SemanticSanitizer/test/internal/trigger"
)

// Client is a programmatic client for SemanticSanitizer.
type Client struct {
	Config *config.SanitizerConfig
}

// New creates a new client instance.
func New(config *config.SanitizerConfig) *Client {
	return &Client{
		Config: config,
	}
}

// Attach executes the client.
// It is meant to be run in a separate goroutine.
func (c *Client) Attach(ctx context.Context, errChan chan error, allRunning chan struct{}) {
	if err := attach.AttachContext(ctx, c.Config, allRunning, nil); err != nil {
		errChan <- fmt.Errorf("attach sanitizer: %w", err)
		return
	}
}

// TestSanitizer tests the sanitizer by running a trigger program against it
// and verifying that the program is killed accordingly.
func (c *Client) TestSanitizer(t *testing.T, triggerProgram []byte) (completed bool, err error) {
	attachErrChan := make(chan error)
	allRunning := make(chan struct{})
	go c.Attach(t.Context(), attachErrChan, allRunning)
	select {
	case err := <-attachErrChan:
		return false, fmt.Errorf("attach sanitizer: %w", err)
	case <-allRunning:
	}
	t.Log("all attached")

	triggerErrChan := make(chan error)
	completedChan := make(chan struct{})
	go trigger.Run(t.Context(), triggerProgram, triggerErrChan, completedChan)

	t.Log("running")

	select {
	case err := <-triggerErrChan:
		return false, err
	case <-completedChan:
		return true, nil
	}
}
