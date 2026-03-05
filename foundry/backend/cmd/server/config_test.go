package main

import "testing"

func TestLoadConfigFromEnv_Defaults(t *testing.T) {
	t.Setenv("PORT", "")
	t.Setenv("LABWIRED_PATH", "")
	t.Setenv("ARTIFACTS_DIR", "")
	t.Setenv("DB_PATH", "")
	t.Setenv("HARDWARE_JSON_PATH", "")
	t.Setenv("KEY_PREFIX_BACKFILL_PATH", "")
	t.Setenv("WORKER_CONCURRENCY", "")
	t.Setenv("MAX_RUN_ATTEMPTS", "")
	t.Setenv("RATE_LIMIT_PER_API_KEY", "")
	t.Setenv("RATE_LIMIT_PER_WORKSPACE", "")
	t.Setenv("RATE_LIMIT_WINDOW_SECONDS", "")
	t.Setenv("WORKSPACE_MAX_INFLIGHT", "")
	t.Setenv("ARTIFACT_RETENTION_DAYS", "")
	t.Setenv("RUN_METADATA_RETENTION_DAYS", "")
	t.Setenv("ARTIFACT_CLEANUP_INTERVAL_SECONDS", "")
	t.Setenv("WORKER_LEASE_TIMEOUT_SECONDS", "")
	t.Setenv("WORKER_HEARTBEAT_INTERVAL_SECONDS", "")
	t.Setenv("ALLOW_INSECURE_STRIPE_WEBHOOKS", "")
	t.Setenv("STRIPE_WEBHOOK_SECRET", "")
	t.Setenv("APP_ENV", "")

	cfg, err := loadConfigFromEnv()
	if err != nil {
		t.Fatalf("loadConfigFromEnv failed: %v", err)
	}

	if cfg.Port != "8080" {
		t.Fatalf("expected default port 8080, got %s", cfg.Port)
	}
	if cfg.ServerOptions.WorkerCount != 4 {
		t.Fatalf("expected default worker count 4, got %d", cfg.ServerOptions.WorkerCount)
	}
	if cfg.ServerOptions.MaxRunAttempts != 3 {
		t.Fatalf("expected default max run attempts 3, got %d", cfg.ServerOptions.MaxRunAttempts)
	}
	if cfg.ServerOptions.RateLimitPerAPIKey != 120 {
		t.Fatalf("expected default API key rate limit 120, got %d", cfg.ServerOptions.RateLimitPerAPIKey)
	}
	if cfg.ServerOptions.RateLimitPerWorkspace != 600 {
		t.Fatalf("expected default workspace rate limit 600, got %d", cfg.ServerOptions.RateLimitPerWorkspace)
	}
	if got := int(cfg.ServerOptions.RateLimitWindow.Seconds()); got != 60 {
		t.Fatalf("expected default rate-limit window 60s, got %d", got)
	}
	if cfg.ServerOptions.MaxInflightPerWorkspace != 8 {
		t.Fatalf("expected default max inflight 8, got %d", cfg.ServerOptions.MaxInflightPerWorkspace)
	}
	if cfg.ServerOptions.ArtifactRetentionDays != 14 {
		t.Fatalf("expected default retention 14, got %d", cfg.ServerOptions.ArtifactRetentionDays)
	}
	if cfg.ServerOptions.RunMetadataRetentionDays != 90 {
		t.Fatalf("expected default metadata retention 90, got %d", cfg.ServerOptions.RunMetadataRetentionDays)
	}
	if got := int(cfg.ServerOptions.CleanupInterval.Seconds()); got != 3600 {
		t.Fatalf("expected default cleanup 3600s, got %d", got)
	}
	if got := int(cfg.ServerOptions.WorkerLeaseTimeout.Seconds()); got != 45 {
		t.Fatalf("expected default lease timeout 45s, got %d", got)
	}
	if got := int(cfg.ServerOptions.WorkerHeartbeatInterval.Seconds()); got != 10 {
		t.Fatalf("expected default heartbeat interval 10s, got %d", got)
	}
}

func TestLoadConfigFromEnv_InvalidNumeric(t *testing.T) {
	t.Setenv("WORKER_CONCURRENCY", "0")
	if _, err := loadConfigFromEnv(); err == nil {
		t.Fatalf("expected error for non-positive worker count")
	}

	t.Setenv("WORKER_CONCURRENCY", "4")
	t.Setenv("ARTIFACT_CLEANUP_INTERVAL_SECONDS", "abc")
	if _, err := loadConfigFromEnv(); err == nil {
		t.Fatalf("expected error for invalid cleanup interval")
	}
	t.Setenv("ARTIFACT_CLEANUP_INTERVAL_SECONDS", "3600")
	t.Setenv("MAX_RUN_ATTEMPTS", "0")
	if _, err := loadConfigFromEnv(); err == nil {
		t.Fatalf("expected error for non-positive max run attempts")
	}
	t.Setenv("MAX_RUN_ATTEMPTS", "3")
	t.Setenv("RATE_LIMIT_PER_API_KEY", "0")
	if _, err := loadConfigFromEnv(); err == nil {
		t.Fatalf("expected error for non-positive API key rate limit")
	}
	t.Setenv("RATE_LIMIT_PER_API_KEY", "120")
	t.Setenv("RATE_LIMIT_PER_WORKSPACE", "0")
	if _, err := loadConfigFromEnv(); err == nil {
		t.Fatalf("expected error for non-positive workspace rate limit")
	}
	t.Setenv("RATE_LIMIT_PER_WORKSPACE", "600")
	t.Setenv("RATE_LIMIT_WINDOW_SECONDS", "0")
	if _, err := loadConfigFromEnv(); err == nil {
		t.Fatalf("expected error for non-positive rate-limit window")
	}
	t.Setenv("RATE_LIMIT_WINDOW_SECONDS", "60")

	t.Setenv("ARTIFACT_CLEANUP_INTERVAL_SECONDS", "60")
	t.Setenv("WORKER_LEASE_TIMEOUT_SECONDS", "10")
	t.Setenv("WORKER_HEARTBEAT_INTERVAL_SECONDS", "10")
	if _, err := loadConfigFromEnv(); err == nil {
		t.Fatalf("expected error when heartbeat interval is not lower than lease timeout")
	}
}

func TestLoadConfigFromEnv_ProductionStripeGuardrails(t *testing.T) {
	t.Setenv("APP_ENV", "production")
	t.Setenv("ALLOW_INSECURE_STRIPE_WEBHOOKS", "true")
	t.Setenv("STRIPE_WEBHOOK_SECRET", "")
	if _, err := loadConfigFromEnv(); err == nil {
		t.Fatalf("expected production guardrail error when insecure mode is true")
	}

	t.Setenv("ALLOW_INSECURE_STRIPE_WEBHOOKS", "false")
	if _, err := loadConfigFromEnv(); err == nil {
		t.Fatalf("expected production guardrail error when webhook secret is missing")
	}

	t.Setenv("STRIPE_WEBHOOK_SECRET", "whsec_test")
	if _, err := loadConfigFromEnv(); err != nil {
		t.Fatalf("expected valid production config, got error: %v", err)
	}
}

func TestValidateRuntimeDependencies(t *testing.T) {
	cfg := config{
		AppEnv:           "development",
		LabWiredPath:     "sh",
		ArtifactsDir:     t.TempDir(),
		HardwareJSONPath: "configs/renode_hardware.json",
	}
	if err := validateRuntimeDependencies(cfg); err != nil {
		t.Fatalf("expected runtime dependency validation to pass, got %v", err)
	}
}
