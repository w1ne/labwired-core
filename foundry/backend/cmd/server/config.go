package main

import (
	"fmt"
	"log"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"github.com/labwired/foundry-backend/internal/api"
)

// config is the validated runtime configuration for the Foundry backend server.
type config struct {
	Port                    string
	BuildCommit             string
	LabWiredPath            string
	ArtifactsDir            string
	DataDir                 string
	DBPath                  string
	HardwareJSONPath        string
	CoreConfigsDir          string
	RepoRootDir             string
	KeyPrefixBackfillPath   string
	AllowInsecureStripeHook bool
	StripeWebhookSecret     string
	AppEnv                  string
	ServerOptions           api.ServerOptions
	ClerkSecretKey          string
	OpenAIAPIKey            string
}

func loadConfigFromEnv() (config, error) {
	cfg := config{
		Port:                  envOrDefault("PORT", "8080"),
		BuildCommit:           envOrDefault("BUILD_COMMIT", envOrDefault("GITHUB_SHA", "unknown")),
		LabWiredPath:          defaultLabWiredPath(),
		ArtifactsDir:          envOrDefault("ARTIFACTS_DIR", "/tmp/foundry/artifacts"),
		DataDir:               envOrDefault("DATA_DIR", "data"),
		DBPath:                envOrDefault("DB_PATH", "foundry.db"),
		HardwareJSONPath:      envOrDefault("HARDWARE_JSON_PATH", "configs/hardware.json"),
		CoreConfigsDir:        envOrDefault("CORE_CONFIGS_DIR", "../../core/configs"),
		RepoRootDir:           defaultRepoRootDir(),
		KeyPrefixBackfillPath: strings.TrimSpace(os.Getenv("KEY_PREFIX_BACKFILL_PATH")),
		StripeWebhookSecret:   strings.TrimSpace(os.Getenv("STRIPE_WEBHOOK_SECRET")),
		AppEnv:                strings.ToLower(strings.TrimSpace(envOrDefault("APP_ENV", "development"))),
		ClerkSecretKey:        strings.TrimSpace(os.Getenv("CLERK_SECRET_KEY")),
		OpenAIAPIKey:          strings.TrimSpace(os.Getenv("OPENAI_API_KEY")),
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
	if cfg.ServerOptions.MaxRunAttempts, err = parsePositiveIntEnv("MAX_RUN_ATTEMPTS", cfg.ServerOptions.MaxRunAttempts); err != nil {
		return config{}, err
	}
	if cfg.ServerOptions.RateLimitPerAPIKey, err = parsePositiveIntEnv("RATE_LIMIT_PER_API_KEY", cfg.ServerOptions.RateLimitPerAPIKey); err != nil {
		return config{}, err
	}
	if cfg.ServerOptions.RateLimitPerWorkspace, err = parsePositiveIntEnv("RATE_LIMIT_PER_WORKSPACE", cfg.ServerOptions.RateLimitPerWorkspace); err != nil {
		return config{}, err
	}
	rateWindowSeconds, err := parsePositiveIntEnv("RATE_LIMIT_WINDOW_SECONDS", int(cfg.ServerOptions.RateLimitWindow.Seconds()))
	if err != nil {
		return config{}, err
	}
	cfg.ServerOptions.RateLimitWindow = time.Duration(rateWindowSeconds) * time.Second
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
	leaseSeconds, err := parsePositiveIntEnv("WORKER_LEASE_TIMEOUT_SECONDS", int(cfg.ServerOptions.WorkerLeaseTimeout.Seconds()))
	if err != nil {
		return config{}, err
	}
	cfg.ServerOptions.WorkerLeaseTimeout = time.Duration(leaseSeconds) * time.Second
	heartbeatSeconds, err := parsePositiveIntEnv("WORKER_HEARTBEAT_INTERVAL_SECONDS", int(cfg.ServerOptions.WorkerHeartbeatInterval.Seconds()))
	if err != nil {
		return config{}, err
	}
	cfg.ServerOptions.WorkerHeartbeatInterval = time.Duration(heartbeatSeconds) * time.Second
	if cfg.ServerOptions.WorkerHeartbeatInterval >= cfg.ServerOptions.WorkerLeaseTimeout {
		return config{}, fmt.Errorf("WORKER_HEARTBEAT_INTERVAL_SECONDS must be lower than WORKER_LEASE_TIMEOUT_SECONDS")
	}

	if err := validateStripeConfig(cfg.AppEnv, cfg.StripeWebhookSecret, cfg.AllowInsecureStripeHook); err != nil {
		return config{}, err
	}

	cfg.ServerOptions.ClerkSecretKey = cfg.ClerkSecretKey
	cfg.ServerOptions.RepoRootDir = cfg.RepoRootDir

	return cfg, nil
}

func envOrDefault(name, defaultValue string) string {
	v := strings.TrimSpace(os.Getenv(name))
	if v == "" {
		return defaultValue
	}
	return v
}

func defaultLabWiredPath() string {
	if v := strings.TrimSpace(os.Getenv("LABWIRED_PATH")); v != "" {
		return v
	}

	candidates := []string{
		"labwired",
		filepath.Clean("../../core/target/debug/labwired"),
		filepath.Clean("../../core/target/release/labwired"),
	}
	for _, candidate := range candidates[1:] {
		if info, err := os.Stat(candidate); err == nil && !info.IsDir() {
			if abs, absErr := filepath.Abs(candidate); absErr == nil {
				return abs
			}
			return candidate
		}
	}
	return candidates[0]
}

func defaultRepoRootDir() string {
	if v := strings.TrimSpace(os.Getenv("REPO_ROOT_DIR")); v != "" {
		return v
	}
	candidate := filepath.Clean("../..")
	if abs, err := filepath.Abs(candidate); err == nil {
		return abs
	}
	return candidate
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
		log.Printf("Warning: STRIPE_WEBHOOK_SECRET is not set in production. Stripe webhook handling will remain disabled until the secret is configured.")
	}
	return nil
}

func validateRuntimeDependencies(cfg config) error {
	if _, err := exec.LookPath(cfg.LabWiredPath); err != nil {
		if cfg.AppEnv == "production" {
			return fmt.Errorf("LABWIRED_PATH command not found: %w", err)
		}
		log.Printf("Warning: LABWIRED_PATH command '%s' not found. Runs will fail.", cfg.LabWiredPath)
	}
	if err := os.MkdirAll(cfg.ArtifactsDir, 0o755); err != nil {
		return fmt.Errorf("ARTIFACTS_DIR is not writable: %w", err)
	}
	if cfg.AppEnv == "production" {
		if _, err := os.Stat(cfg.HardwareJSONPath); err != nil {
			return fmt.Errorf("HARDWARE_JSON_PATH must exist in production: %w", err)
		}
		if info, err := os.Stat(cfg.CoreConfigsDir); err != nil {
			return fmt.Errorf("CORE_CONFIGS_DIR must exist in production: %w", err)
		} else if !info.IsDir() {
			return fmt.Errorf("CORE_CONFIGS_DIR must be a directory in production: %s", cfg.CoreConfigsDir)
		}
	}
	logOptionalDependencyState(cfg)
	return nil
}

func logOptionalDependencyState(cfg config) {
	integrations := []struct {
		name    string
		enabled bool
		reason  string
	}{
		{
			name:    "stripe_webhooks",
			enabled: cfg.StripeWebhookSecret != "",
			reason:  "STRIPE_WEBHOOK_SECRET is unset; /v1/webhooks/stripe will reject requests until configured.",
		},
		{
			name:    "clerk_account_api",
			enabled: cfg.ClerkSecretKey != "",
			reason:  "CLERK_SECRET_KEY is unset; account-scoped routes will remain unavailable.",
		},
		{
			name:    "openai_synthesis",
			enabled: cfg.OpenAIAPIKey != "",
			reason:  "OPENAI_API_KEY is unset; AI synthesis features will remain unavailable.",
		},
	}

	for _, integration := range integrations {
		if integration.enabled {
			log.Printf("Optional integration enabled: %s", integration.name)
			continue
		}
		log.Printf("Warning: optional integration disabled: %s. %s", integration.name, integration.reason)
	}
}
