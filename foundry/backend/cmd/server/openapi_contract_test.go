package main

import (
	"os"
	"path/filepath"
	"reflect"
	"testing"

	"gopkg.in/yaml.v3"
)

func TestOpenAPI_StaticAndRootSpecsStayInSync(t *testing.T) {
	staticSpecPath := filepath.Join("..", "..", "static", "openapi.yaml")
	rootSpecPath := filepath.Join("..", "..", "openapi.yaml")

	staticSpec, err := os.ReadFile(staticSpecPath)
	if err != nil {
		t.Fatalf("read static openapi spec failed: %v", err)
	}
	rootSpec, err := os.ReadFile(rootSpecPath)
	if err != nil {
		t.Fatalf("read root openapi spec failed: %v", err)
	}

	if !reflect.DeepEqual(staticSpec, rootSpec) {
		t.Fatalf("openapi specs diverged: %s and %s must stay identical", staticSpecPath, rootSpecPath)
	}
}

func TestOpenAPI_VerifyAndRunStatusContract(t *testing.T) {
	specPath := filepath.Join("..", "..", "static", "openapi.yaml")
	data, err := os.ReadFile(specPath)
	if err != nil {
		t.Fatalf("read openapi spec failed: %v", err)
	}

	var doc map[string]any
	if err := yaml.Unmarshal(data, &doc); err != nil {
		t.Fatalf("parse openapi spec failed: %v", err)
	}

	components := getMap(t, doc, "components")
	schemas := getMap(t, components, "schemas")

	verificationRequest := getMap(t, schemas, "VerificationRequest")
	required := getStringSlice(t, verificationRequest, "required")
	if !contains(required, "chip_yaml") {
		t.Fatalf("VerificationRequest.required must include chip_yaml, got: %v", required)
	}

	verifyProps := getMap(t, getMap(t, verificationRequest, "properties"), "chip_yaml")
	if verifyProps["type"] != "string" {
		t.Fatalf("VerificationRequest.properties.chip_yaml.type must be string")
	}

	runStatus := getMap(t, schemas, "RunStatus")
	runStatusProps := getMap(t, runStatus, "properties")
	for _, field := range []string{"run_id", "status", "assertions_passed", "assertions_total", "created_at"} {
		if _, ok := runStatusProps[field]; !ok {
			t.Fatalf("RunStatus.properties missing required runtime field %q", field)
		}
	}

	paths := getMap(t, doc, "paths")
	verifyPath := getMap(t, paths, "/models/verify")
	verifyPost := getMap(t, verifyPath, "post")
	requestBody := getMap(t, verifyPost, "requestBody")
	content := getMap(t, requestBody, "content")
	appJSON := getMap(t, content, "application/json")
	schema := getMap(t, appJSON, "schema")
	ref, ok := schema["$ref"].(string)
	if !ok {
		t.Fatalf("/models/verify request schema must contain a string $ref")
	}
	if ref != "#/components/schemas/VerificationRequest" {
		t.Fatalf("unexpected /models/verify request schema ref: %s", ref)
	}
}

func getMap(t *testing.T, in map[string]any, key string) map[string]any {
	t.Helper()
	v, ok := in[key]
	if !ok {
		t.Fatalf("missing key %q", key)
	}
	out, ok := v.(map[string]any)
	if !ok {
		t.Fatalf("key %q is not an object", key)
	}
	return out
}

func getStringSlice(t *testing.T, in map[string]any, key string) []string {
	t.Helper()
	v, ok := in[key]
	if !ok {
		return nil
	}
	raw, ok := v.([]any)
	if !ok {
		t.Fatalf("key %q is not an array", key)
	}
	out := make([]string, 0, len(raw))
	for _, item := range raw {
		s, ok := item.(string)
		if !ok {
			t.Fatalf("key %q contains non-string array item", key)
		}
		out = append(out, s)
	}
	return out
}

func contains(items []string, target string) bool {
	for _, item := range items {
		if item == target {
			return true
		}
	}
	return false
}
