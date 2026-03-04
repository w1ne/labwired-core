package db

import (
	"fmt"
	"log"

	"github.com/google/uuid"
	"golang.org/x/crypto/bcrypt"
)

type APIKey struct {
	ID          string `json:"id"`
	KeyHash     string `json:"-"`
	WorkspaceID string `json:"workspace_id"`
	Tier        string `json:"tier"`
}

func (s *Store) CreateKey(workspaceID, plaintextKey string) (*APIKey, error) {
	hash, err := bcrypt.GenerateFromPassword([]byte(plaintextKey), bcrypt.DefaultCost)
	if err != nil {
		return nil, err
	}

	id := uuid.New().String()
	_, err = s.db.Exec(
		"INSERT INTO api_keys (id, key_hash, workspace_id, tier) VALUES (?, ?, ?, ?)",
		id, string(hash), workspaceID, "builder",
	)
	if err != nil {
		return nil, err
	}

	return &APIKey{
		ID:          id,
		KeyHash:     string(hash),
		WorkspaceID: workspaceID,
		Tier:        "builder",
	}, nil
}

func (s *Store) ValidateKey(plaintextKey string) (*APIKey, error) {
	rows, err := s.db.Query("SELECT id, key_hash, workspace_id, tier FROM api_keys WHERE revoked = FALSE")
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	for rows.Next() {
		var k APIKey
		if err := rows.Scan(&k.ID, &k.KeyHash, &k.WorkspaceID, &k.Tier); err != nil {
			log.Printf("DB error scanning key: %v", err)
			continue
		}

		if err := bcrypt.CompareHashAndPassword([]byte(k.KeyHash), []byte(plaintextKey)); err == nil {
			return &k, nil
		}
	}

	return nil, fmt.Errorf("invalid api key")
}
