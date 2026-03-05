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
	t.Setenv("WORKSPACE_MAX_INFLIGHT", "")
	t.Setenv("ARTIFACT_RETENTION_DAYS", "")
	t.Setenv("RUN_METADATA_RETENTION_DAYS", "")
	t.Setenv("ARTIFACT_CLEANUP_INTERVAL_SECONDS", "")
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
