package synthesis

import "strings"

func isBoardRequest(req Request) bool {
	if req.Kind == "board_onboarding" {
		return true
	}
	if req.Kind == "peripheral_model_ingest" {
		return false
	}
	lower := strings.ToLower(req.ComponentName + " " + req.Requirements)
	boardTokens := []string{
		"board", "nucleo", "mb", "bring-up", "bringup", "led", "button", "uart debug",
		"system manifest", "board onboarding", "proof", "example manifest",
	}
	for _, token := range boardTokens {
		if strings.Contains(lower, token) {
			return true
		}
	}
	return false
}

func boardValidationPlan(req Request) []string {
	plan := []string{}
	targets := map[string]bool{}
	for _, target := range req.ValidationTargets {
		targets[strings.ToLower(strings.TrimSpace(target))] = true
	}
	if len(targets) == 0 {
		return []string{
			"Boot firmware on app core and verify PC/SP initialization.",
			"Confirm deterministic UART smoke output on the board console path.",
			"Toggle board LED and exercise user button mapping in the simulator manifest.",
			"Run unsupported-instruction audit against the selected smoke firmware.",
		}
	}
	if targets["uart_smoke"] {
		plan = append(plan, "Confirm deterministic UART smoke output on the board console path.")
	}
	if targets["boot"] {
		plan = append(plan, "Boot firmware on app core and verify PC/SP initialization.")
	}
	if targets["led_smoke"] || targets["button_smoke"] || targets["io_smoke"] {
		plan = append(plan, "Toggle board LED and exercise user button mapping in the simulator manifest.")
	}
	if targets["unsupported_instruction_audit"] {
		plan = append(plan, "Run unsupported-instruction audit against the selected smoke firmware.")
	}
	if len(plan) == 0 {
		plan = append(plan, "Produce the requested board validation evidence declared in validation_targets.")
	}
	return plan
}

func requestedBoardCapabilities(req Request) []string {
	if len(req.DesiredCapabilities) == 0 {
		return []string{"boot", "uart_console", "led_control", "button_input"}
	}
	return nonEmptyCopy(req.DesiredCapabilities)
}

func validatedBoardCapabilities(req Request) []string {
	requested := requestedBoardCapabilities(req)
	targets := map[string]bool{}
	for _, target := range req.ValidationTargets {
		targets[strings.ToLower(strings.TrimSpace(target))] = true
	}
	if len(targets) == 0 {
		return requested
	}
	validated := []string{}
	for _, capability := range requested {
		switch strings.ToLower(strings.TrimSpace(capability)) {
		case "boot":
			if targets["boot"] || targets["uart_smoke"] {
				validated = append(validated, capability)
			}
		case "uart_console":
			if targets["uart_smoke"] {
				validated = append(validated, capability)
			}
		case "led_control", "button_input":
			if targets["io_smoke"] || targets["led_smoke"] || targets["button_smoke"] {
				validated = append(validated, capability)
			}
		default:
			if targets["contract_check"] {
				validated = append(validated, capability)
			}
		}
	}
	return validated
}

func nonEmptyCopy(values []string) []string {
	out := make([]string, 0, len(values))
	for _, value := range values {
		if trimmed := strings.TrimSpace(value); trimmed != "" {
			out = append(out, trimmed)
		}
	}
	return out
}

func boardPromotionMode(req Request) string {
	if req.Constraints != nil && req.Constraints.MustWriteRepoAssets {
		return "apply_to_repo"
	}
	return "artifact_only"
}

func inferChipGuess(req Request) string {
	input := req.ComponentName + " " + req.Requirements
	if req.Board != nil {
		input += " " + req.Board.MCU + " " + req.Board.MarketingName + " " + req.Board.BoardID
	}
	lower := strings.ToLower(input)
	switch {
	case strings.Contains(lower, "stm32wba52"):
		return "stm32wba52"
	case strings.Contains(lower, "stm32wba"):
		return "stm32wba"
	case strings.Contains(lower, "stm32wb55"):
		return "stm32wb55"
	case strings.Contains(lower, "stm32wb"):
		return "stm32wb"
	case strings.Contains(lower, "stm32f411"):
		return "stm32f411re"
	case strings.Contains(lower, "nrf52840"):
		return "nrf52840"
	default:
		return ""
	}
}

func inferBoardID(req Request) string {
	if req.Board != nil && strings.TrimSpace(req.Board.BoardID) != "" {
		return SanitizeIdent(req.Board.BoardID)
	}
	if req.Board != nil && strings.TrimSpace(req.Board.MarketingName) != "" {
		return SanitizeIdent(req.Board.MarketingName)
	}
	lower := strings.ToLower(req.ComponentName)
	switch {
	case strings.Contains(lower, "mb1355"):
		return "mb1355c"
	case strings.Contains(lower, "nucleo-wba52cg"):
		return "nucleo_wba52cg"
	case strings.Contains(lower, "nucleo-wb55rg"):
		return "nucleo-wb55rg"
	default:
		return SanitizeIdent(req.ComponentName)
	}
}
