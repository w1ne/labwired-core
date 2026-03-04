package main

import (
	"crypto/rand"
	"encoding/hex"
	"flag"
	"fmt"
	"log"

	"github.com/labwired/foundry-backend/internal/db"
)

func generateSecureToken() string {
	b := make([]byte, 16)
	if _, err := rand.Read(b); err != nil {
		log.Fatal(err)
	}
	return "lw_sk_live_" + hex.EncodeToString(b)
}

func main() {
	dbPath := flag.String("db", "foundry.db", "Path to SQLite database")
	workspaceID := flag.String("workspace", "default-workspace", "Workspace ID to bind the key to")
	flag.Parse()

	store, err := db.NewStore(*dbPath)
	if err != nil {
		log.Fatalf("Failed to open DB: %v", err)
	}

	rawKey := generateSecureToken()
	apiKey, err := store.CreateKey(*workspaceID, rawKey)
	if err != nil {
		log.Fatalf("Failed to create key: %v", err)
	}

	fmt.Printf("✅ API Key Created Successfully!\n")
	fmt.Printf("Workspace ID : %s\n", apiKey.WorkspaceID)
	fmt.Printf("Key ID       : %s\n", apiKey.ID)
	fmt.Printf("Tier         : %s\n", apiKey.Tier)
	fmt.Printf("\nYour API Key : %s\n", rawKey)
	fmt.Printf("⚠️  SAVE THIS KEY NOW. The plaintext is not stored and cannot be retrieved.\n")
}
