package synthesis

import (
	"regexp"
	"strconv"
	"strings"
)

func ParseHex(s any) int64 {
	switch v := s.(type) {
	case int:
		return int64(v)
	case int64:
		return v
	case string:
		trimmed := strings.TrimSpace(v)
		if trimmed == "" || strings.ToLower(trimmed) == "n/a" {
			return 0
		}
		// Binary '00001010'
		if len(trimmed) == 8 {
			if matched, _ := regexp.MatchString("^[01]+$", trimmed); matched {
				val, _ := strconv.ParseInt(trimmed, 2, 64)
				return val
			}
		}
		if strings.HasPrefix(strings.ToLower(trimmed), "0x") {
			val, _ := strconv.ParseInt(trimmed[2:], 16, 64)
			return val
		}
		val, _ := strconv.ParseInt(trimmed, 10, 64)
		return val
	}
	return 0
}

func SanitizeIdent(s string) string {
	if s == "" {
		return "unknown"
	}
	s = strings.ToLower(s)
	s = strings.ReplaceAll(s, " ", "_")
	s = strings.ReplaceAll(s, "-", "_")
	s = strings.ReplaceAll(s, "[", "_")
	s = strings.ReplaceAll(s, "]", "_")

	reg := regexp.MustCompile("[^a-z0-9_]")
	s = reg.ReplaceAllString(s, "")

	if len(s) > 0 && s[0] >= '0' && s[0] <= '9' {
		s = "reg_" + s
	}
	if s == "" {
		return "unknown"
	}
	return s
}

func MapAccess(s string) string {
	s = strings.ToLower(s)
	if strings.Contains(s, "readwrite") || strings.Contains(s, "r/w") {
		return "ReadWrite"
	}
	if strings.Contains(s, "readonly") || strings.Contains(s, "ro") {
		return "ReadOnly"
	}
	if strings.Contains(s, "writeonly") || strings.Contains(s, "wo") {
		return "WriteOnly"
	}
	return "ReadWrite" // Default
}

func Convert(input YamlPeripheral) StrictIR {
	registers := make([]IRRegister, 0, len(input.Registers))
	interruptMapping := make(map[string]int)
	_ = 10 // Placeholder for irqCounter

	for _, reg := range input.Registers {
		fields := make([]IRField, 0, len(reg.Fields))
		maxBit := 0
		for _, f := range reg.Fields {
			low, high := f.BitRange[0], f.BitRange[1]
			if low > high {
				low, high = high, low
			}
			fields = append(fields, IRField{
				Name:      SanitizeIdent(f.Name),
				BitOffset: low,
				BitWidth:  high - low + 1,
				Description: f.Description,
			})
			if high+1 > maxBit {
				maxBit = high + 1
			}
		}

		size := 32
		if maxBit <= 8 {
			size = 8
		} else if maxBit <= 16 {
			size = 16
		}

		registers = append(registers, IRRegister{
			Name:       SanitizeIdent(reg.Name),
			Offset:     ParseHex(reg.Offset),
			Size:       size,
			Access:     MapAccess(reg.Access),
			ResetValue: ParseHex(reg.ResetValue),
			Fields:     fields,
			Description: reg.Description,
		})
	}

	// Porting of TimingHooks/Heuristics would go here...
    // For now, return the basic IR
	return StrictIR{
		Name:             SanitizeIdent(input.Name),
		Arch:             "Arm",
		Description:      "AI Generated Device for Codegen",
		Peripherals: map[string]IRPeripheral{
			SanitizeIdent(input.Name): {
				Name:        SanitizeIdent(input.Name),
				BaseAddress: 0x40000000,
				Registers:   registers,
				Interrupts:  []IRInterrupt{},
				Timing:      []IRTimingHook{},
			},
		},
		InterruptMapping: interruptMapping,
	}
}
