package main

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"log"
	"net/http"
	"os"
	"os/signal"
	"strings"
	"syscall"
	"time"

	"github.com/labwired/foundry-backend/internal/api"
	"github.com/labwired/foundry-backend/internal/catalog"
	"github.com/labwired/foundry-backend/internal/db"
	"github.com/labwired/foundry-backend/internal/verification"
)

func main() {
	cfg, err := loadConfigFromEnv()
	if err != nil {
		log.Fatalf("Invalid server configuration: %v", err)
	}
	if err := validateRuntimeDependencies(cfg); err != nil {
		log.Fatalf("Runtime dependency validation failed: %v", err)
	}

	store, err := db.NewStore(cfg.DBPath)
	if err != nil {
		log.Fatalf("Failed to initialize database: %v", err)
	}
	defer func() {
		if err := store.Close(); err != nil {
			log.Printf("Failed to close database: %v", err)
		}
	}()

	var hwItems []db.HardwareItem
	hwData, err := os.ReadFile(cfg.HardwareJSONPath)
	if err != nil {
		if cfg.AppEnv == "production" {
			log.Fatalf("Failed to read required hardware config from %s: %v", cfg.HardwareJSONPath, err)
		}
		log.Printf("Warning: failed to read hardware config from %s: %v", cfg.HardwareJSONPath, err)
	} else {
		if err := json.Unmarshal(hwData, &hwItems); err != nil {
			log.Printf("Warning: failed to parse hardware config: %v", err)
		} else {
			if err := store.SeedHardware(hwItems); err != nil {
				log.Printf("Warning: failed to seed hardware catalog: %v", err)
			} else {
				log.Printf("Successfully seeded %d hardware items from %s", len(hwItems), cfg.HardwareJSONPath)
			}
		}
	}

	if cfg.KeyPrefixBackfillPath != "" {
		plaintextKeys, err := readKeyLines(cfg.KeyPrefixBackfillPath)
		if err != nil {
			log.Fatalf("Failed to read KEY_PREFIX_BACKFILL_PATH file: %v", err)
		}
		updated, err := store.BackfillKeyPrefixes(plaintextKeys)
		if err != nil {
			log.Fatalf("Failed key prefix backfill: %v", err)
		}
		log.Printf("Key prefix backfill completed: updated %d row(s)", updated)
	}

	cat := catalog.NewManager(store)
	if err := cat.RebuildGitBackedCatalog(cfg.CoreConfigsDir, hwItems); err != nil {
		log.Printf("Warning: failed to rebuild git-backed catalog: %v", err)
	} else {
		log.Printf("Catalog rebuild imported %d hardware index entries", len(hwItems))
	}
	log.Printf(
		"Foundry startup: build_commit=%s app_env=%s hardware_endpoint_source=catalog core_configs_dir=%s",
		cfg.BuildCommit,
		cfg.AppEnv,
		cfg.CoreConfigsDir,
	)

	orch := verification.NewOrchestrator(cfg.LabWiredPath)
	srv := api.NewServer(orch, store, cat, cfg.ArtifactsDir, cfg.DataDir, cfg.ServerOptions)

	httpServer := &http.Server{
		Addr:              ":" + cfg.Port,
		Handler:           srv,
		ReadHeaderTimeout: 5 * time.Second,
		ReadTimeout:       15 * time.Second,
		WriteTimeout:      60 * time.Second,
		IdleTimeout:       60 * time.Second,
		MaxHeaderBytes:    1 << 20,
	}

	log.Printf("Foundry Backend listening on port %s", cfg.Port)
	go func() {
		if err := httpServer.ListenAndServe(); err != nil && !errors.Is(err, http.ErrServerClosed) {
			log.Fatalf("Server failed: %v", err)
		}
	}()

	stop := make(chan os.Signal, 1)
	signal.Notify(stop, os.Interrupt, syscall.SIGTERM)
	<-stop
	log.Println("Shutdown signal received")

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	if err := httpServer.Shutdown(ctx); err != nil {
		log.Fatalf("Graceful shutdown failed: %v", err)
	}

	workerCtx, workerCancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer workerCancel()
	if err := srv.Shutdown(workerCtx); err != nil {
		log.Printf("Worker drain shutdown incomplete: %v", err)
	}
	log.Println("Server shutdown complete")
}

func readKeyLines(path string) ([]string, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer f.Close()

	var out []string
	sc := bufio.NewScanner(f)
	for sc.Scan() {
		line := strings.TrimSpace(sc.Text())
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		out = append(out, line)
	}
	if err := sc.Err(); err != nil {
		return nil, err
	}
	return out, nil
}
