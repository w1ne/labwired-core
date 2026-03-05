package main

import (
	"fmt"
	"os"
	"strconv"
	"strings"
	"time"

	"github.com/labwired/foundry-backend/internal/api"
)

// config is the validated runtime configuration for the Foundry backend server.
type config struct {
	Port                    string
	LabWiredPath            string
	ArtifactsDir            string
	DBPath                  string
	HardwareJSONPath        string
	KeyPrefixBackfillPath   string
	AllowInsecureStripeHook bool
	StripeWebhookSecret     string
	AppEnv                  string
	ServerOptions           api.ServerOptions
}

func loadConfigFromEnv() (config, error) {
	cfg := config{
		Port:                  envOrDefault("PORT", "8080"),
		LabWiredPath:          envOrDefault("LABWIRED_PATH", "labwired"),
		ArtifactsDir:          envOrDefault("ARTIFACTS_DIR", "/tmp/foundry/artifacts"),
		DBPath:                envOrDefault("DB_PATH", "foundry.db"),
		HardwareJSONPath:      envOrDefault("HARDWARE_JSON_PATH", "configs/renode_hardware.json"),
		KeyPrefixBackfillPath: strings.TrimSpace(os.Getenv("KEY_PREFIX_BACKFILL_PATH")),
		StripeWebhookSecret:   strings.TrimSpace(os.Getenv("STRIPE_WEBHOOK_SECRET")),
		AppEnv:                strings.ToLower(strings.TrimSpace(envOrDefault("APP_ENV", "development"))),
		ServerOptions:         api.DefaultServerOptions(),
	}

	allowInsecure, err := parseBoolEnv("ALLOW_INSECURE_STRIPE_WEBHOOKS", false)
	if err != nil {
		return config{}, err
	}
	cfg.AllowInsecureStripeHook = allowInsecure

	if cfg.ServerOptions.WorkerCount, err = parsePositiveIntEnv("WORKER_CONCURRENCY", cfg.ServerOptions.WorkerCount); err != nil {
		return config{}, err
	}
	if cfg.ServerOptions.MaxInflightPerWorkspace, err = parsePositiveIntEnv("WORKSPACE_MAX_INFLIGHT", cfg.ServerOptions.MaxInflightPerWorkspace); err != nil {
		return config{}, err
	}
	if cfg.ServerOptions.ArtifactRetentionDays, err = parsePositiveIntEnv("ARTIFACT_RETENTION_DAYS", cfg.ServerOptions.ArtifactRetentionDays); err != nil {
		return config{}, err
	}
	if cfg.ServerOptions.RunMetadataRetentionDays, err = parsePositiveIntEnv("RUN_METADATA_RETENTION_DAYS", cfg.ServerOptions.RunMetadataRetentionDays); err != nil {
		return config{}, err
	}
	cleanupSeconds, err := parsePositiveIntEnv("ARTIFACT_CLEANUP_INTERVAL_SECONDS", int(cfg.ServerOptions.CleanupInterval.Seconds()))
	if err != nil {
		return config{}, err
	}
	cfg.ServerOptions.CleanupInterval = time.Duration(cleanupSeconds) * time.Second

	if err := validateStripeConfig(cfg.AppEnv, cfg.StripeWebhookSecret, cfg.AllowInsecureStripeHook); err != nil {
		return config{}, err
	}

	return cfg, nil
}

func envOrDefault(name, defaultValue string) string {
	v := strings.TrimSpace(os.Getenv(name))
	if v == "" {
		return defaultValue
	}
	return v
}

func parsePositiveIntEnv(name string, defaultValue int) (int, error) {
	raw := strings.TrimSpace(os.Getenv(name))
	if raw == "" {
		return defaultValue, nil
	}
	parsed, err := strconv.Atoi(raw)
	if err != nil {
		return 0, fmt.Errorf("%s must be an integer: %w", name, err)
	}
	if parsed <= 0 {
		return 0, fmt.Errorf("%s must be > 0", name)
	}
	return parsed, nil
}

func parseBoolEnv(name string, defaultValue bool) (bool, error) {
	raw := strings.TrimSpace(os.Getenv(name))
	if raw == "" {
		return defaultValue, nil
	}
	parsed, err := strconv.ParseBool(raw)
	if err != nil {
		return false, fmt.Errorf("%s must be a boolean: %w", name, err)
	}
	return parsed, nil
}

func validateStripeConfig(appEnv, webhookSecret string, allowInsecure bool) error {
	if appEnv != "production" {
		return nil
	}
	if allowInsecure {
		return fmt.Errorf("ALLOW_INSECURE_STRIPE_WEBHOOKS must be false in production")
	}
	if webhookSecret == "" {
		return fmt.Errorf("STRIPE_WEBHOOK_SECRET is required in production")
	}
	return nil
}
