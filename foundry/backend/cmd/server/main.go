package main

import (
	"log"
	"net/http"
	"os"

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

	cat := catalog.NewManager()
	orch := verification.NewOrchestrator(labwiredPath)
	srv := api.NewServer(orch, store, cat, artifactsDir)

	log.Printf("Foundry Backend listening on port %s", port)
	if err := http.ListenAndServe(":"+port, srv); err != nil {
		log.Fatalf("Server failed: %v", err)
	}
}
