package verification

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

type Result struct {
	Pass             bool   `json:"pass"`
	AssertionsPassed int    `json:"assertions_passed"`
	AssertionsTotal  int    `json:"assertions_total"`
	VcdURL           string `json:"vcd_url,omitempty"`
	IrURL            string `json:"ir_url"`
	Error            string `json:"error,omitempty"`
}

type Orchestrator struct {
	LabwiredPath string
}

func NewOrchestrator(labwiredPath string) *Orchestrator {
	return &Orchestrator{
		LabwiredPath: labwiredPath,
	}
}

// RunSimulation executes the labwired CLI against the submitted IR file and writes
// output artifacts (result.json, proof.vcd) into artifactDir.
func (o *Orchestrator) RunSimulation(ctx context.Context, irPath, artifactDir string) (*Result, error) {
	if err := os.MkdirAll(artifactDir, 0755); err != nil {
		return nil, fmt.Errorf("failed to create artifact dir: %w", err)
	}

	// labwired test --ir <path> --vcd --output-dir <dir>
	cmd := exec.CommandContext(ctx, o.LabwiredPath,
		"test",
		"--ir", irPath,
		"--vcd",
		"--output-dir", artifactDir,
	)
	cmd.Dir = artifactDir

	output, err := cmd.CombinedOutput()
	if err != nil {
		// Write the error log so it can be served as an artifact.
		_ = os.WriteFile(filepath.Join(artifactDir, "error.log"), output, 0644)
		return &Result{
			Pass:  false,
			Error: string(output),
		}, nil
	}

	// Try to parse result.json written by the CLI.
	resultPath := filepath.Join(artifactDir, "result.json")
	if data, err := os.ReadFile(resultPath); err == nil {
		var parsed struct {
			Pass             bool `json:"pass"`
			AssertionsPassed int  `json:"assertions_passed"`
			AssertionsTotal  int  `json:"assertions_total"`
		}
		if json.Unmarshal(data, &parsed) == nil {
			return &Result{
				Pass:             parsed.Pass,
				AssertionsPassed: parsed.AssertionsPassed,
				AssertionsTotal:  parsed.AssertionsTotal,
				IrURL:            fmt.Sprintf("/artifacts/%s/output.json", filepath.Base(artifactDir)),
				VcdURL:           fmt.Sprintf("/artifacts/%s/proof.vcd", filepath.Base(artifactDir)),
			}, nil
		}
	}

	// Fallback: parse assertions from stdout (e.g., "49/49 assertions passed").
	passed, total := parseAssertions(string(output))
	return &Result{
		Pass:             total > 0 && passed == total,
		AssertionsPassed: passed,
		AssertionsTotal:  total,
		IrURL:            fmt.Sprintf("/artifacts/%s/output.json", filepath.Base(artifactDir)),
		VcdURL:           fmt.Sprintf("/artifacts/%s/proof.vcd", filepath.Base(artifactDir)),
	}, nil
}

// parseAssertions extracts passed/total counts from labwired CLI stdout.
// Expects a line like "49/49 assertions passed".
func parseAssertions(output string) (passed, total int) {
	for _, line := range strings.Split(output, "\n") {
		var p, t int
		if n, _ := fmt.Sscanf(strings.TrimSpace(line), "%d/%d assertions passed", &p, &t); n == 2 {
			return p, t
		}
	}
	return 0, 0
}
