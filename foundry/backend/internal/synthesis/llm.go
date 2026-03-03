package synthesis

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"strings"

	"github.com/sashabaranov/go-openai"
)

type LLMClient struct {
	client *openai.Client
	model  string
}

func NewLLMClient() *LLMClient {
	apiKey := os.Getenv("XAI_API_KEY")
	baseURL := os.Getenv("XAI_BASE_URL")
	if baseURL == "" {
		baseURL = "https://api.x.ai/v1"
	}

	config := openai.DefaultConfig(apiKey)
	config.BaseURL = baseURL

	return &LLMClient{
		client: openai.NewClientWithConfig(config),
		model:  "grok-4-1-fast-reasoning",
	}
}

func (c *LLMClient) Complete(ctx context.Context, systemPrompt, userPrompt string) (string, error) {
	resp, err := c.client.CreateChatCompletion(
		ctx,
		openai.ChatCompletionRequest{
			Model: c.model,
			Messages: []openai.ChatCompletionMessage{
				{
					Role:    openai.ChatMessageRoleSystem,
					Content: systemPrompt,
				},
				{
					Role:    openai.ChatMessageRoleUser,
					Content: userPrompt,
				},
			},
		},
	)
	if err != nil {
		return "", err
	}

	return resp.Choices[0].Message.Content, nil
}

func (c *LLMClient) DiscoverRegisters(ctx context.Context, text string) ([]map[string]any, error) {
	return nil, fmt.Errorf("DiscoverRegisters not fully implemented")
}

func (c *LLMClient) ExtractRegisterFields(ctx context.Context, text, registerName string) (map[string]any, error) {
	systemPrompt := "You are an expert embedded systems engineer. You excel at mapping bits to functions. Accuracy is paramount for simulation correctness."
	userPrompt := fmt.Sprintf(`Task: Extract the bitfield mapping for the register: %s.

Step 1: Find the bit definition table or description for %s.
Step 2: Identify each bit/field.
Step 3: Determine bit_range [start, end], access (ReadWrite, ReadOnly, WriteOnly), and reset_value.

Text:
%s

Respond ONLY with a JSON object:
{
    "name": "%s",
    "offset": "0x??",
    "reset_value": "0x??",
    "access": "ReadWrite",
    "fields": [
        {
            "name": "FIELD",
            "bit_range": [0, 0],
            "description": "Functional description"
        }
    ]
}`, registerName, registerName, text, registerName)

	resp, err := c.Complete(ctx, systemPrompt, userPrompt)
	if err != nil {
		return nil, err
	}

	return parseJSON[map[string]any](resp)
}

func (c *LLMClient) ExtractBehavior(ctx context.Context, text string, contextBlob any) ([]map[string]any, error) {
	systemPrompt := "You are an expert hardware simulation engineer. Your goal is to detect side effects and causal logic that standard SVD files miss."

	contextJSON, _ := json.MarshalIndent(contextBlob, "", "  ")

	userPrompt := fmt.Sprintf(`Context (Known Registers and Fields):
%s

Task: Deeply analyze the datasheet text to synthesize simulation behaviors (Timing Hooks).
Respond ONLY with a JSON list of TimingDescriptor objects.

Text:
%s`, string(contextJSON), text)

	resp, err := c.Complete(ctx, systemPrompt, userPrompt)
	if err != nil {
		return nil, err
	}

	return parseJSON[[]map[string]any](resp)
}

func parseJSON[T any](s string) (T, error) {
	var res T
	s = strings.TrimSpace(s)
	if strings.HasPrefix(s, "```json") {
		s = strings.TrimPrefix(s, "```json")
		s = strings.TrimSuffix(s, "```")
	} else if strings.HasPrefix(s, "```") {
		s = strings.TrimPrefix(s, "```")
		s = strings.TrimSuffix(s, "```")
	}
	s = strings.TrimSpace(s)
	err := json.Unmarshal([]byte(s), &res)
	return res, err
}
