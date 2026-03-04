package main

import (
	"context"
	"errors"
	"log"
	"net/http"
	"os"
	"os/signal"
	"syscall"
	"time"

	"github.com/labwired/foundry-backend/internal/api"
	"github.com/labwired/foundry-backend/internal/catalog"
	"github.com/labwired/foundry-backend/internal/db"
	"github.com/labwired/foundry-backend/internal/verification"
)

func main() {
	port := os.Getenv("PORT")
	if port == "" {
		port = "8080"
	}

	labwiredPath := os.Getenv("LABWIRED_PATH")
	if labwiredPath == "" {
		labwiredPath = "labwired"
	}

	artifactsDir := os.Getenv("ARTIFACTS_DIR")
	if artifactsDir == "" {
		artifactsDir = "/tmp/foundry/artifacts"
	}

	dbPath := os.Getenv("DB_PATH")
	if dbPath == "" {
		dbPath = "foundry.db"
	}

	store, err := db.NewStore(dbPath)
	if err != nil {
		log.Fatalf("Failed to initialize database: %v", err)
	}
	defer func() {
		if err := store.Close(); err != nil {
			log.Printf("Failed to close database: %v", err)
		}
	}()

	cat := catalog.NewManager()
	orch := verification.NewOrchestrator(labwiredPath)
	srv := api.NewServer(orch, store, cat, artifactsDir)

	httpServer := &http.Server{
		Addr:              ":" + port,
		Handler:           srv,
		ReadHeaderTimeout: 5 * time.Second,
		ReadTimeout:       15 * time.Second,
		WriteTimeout:      60 * time.Second,
		IdleTimeout:       60 * time.Second,
		MaxHeaderBytes:    1 << 20,
	}

	log.Printf("Foundry Backend listening on port %s", port)
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
