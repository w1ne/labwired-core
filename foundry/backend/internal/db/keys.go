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
	KeyPrefix   string `json:"-"`
	WorkspaceID string `json:"workspace_id"`
	Tier        string `json:"tier"`
}

func keyPrefix(plaintextKey string) string {
	const prefixLen = 16
	if len(plaintextKey) <= prefixLen {
		return plaintextKey
	}
	return plaintextKey[:prefixLen]
}

func (s *Store) CreateKey(workspaceID, plaintextKey string) (*APIKey, error) {
	hash, err := bcrypt.GenerateFromPassword([]byte(plaintextKey), bcrypt.DefaultCost)
	if err != nil {
		return nil, err
	}

	id := uuid.New().String()
	prefix := keyPrefix(plaintextKey)
	_, err = s.db.Exec(
		"INSERT INTO api_keys (id, key_hash, key_prefix, workspace_id, tier) VALUES (?, ?, ?, ?, ?)",
		id, string(hash), prefix, workspaceID, "builder",
	)
	if err != nil {
		return nil, err
	}

	return &APIKey{
		ID:          id,
		KeyHash:     string(hash),
		KeyPrefix:   prefix,
		WorkspaceID: workspaceID,
		Tier:        "builder",
	}, nil
}

func (s *Store) ValidateKey(plaintextKey string) (*APIKey, error) {
	prefix := keyPrefix(plaintextKey)

	// Fast path: narrow candidates via indexed prefix.
	rows, err := s.db.Query(
		"SELECT id, key_hash, key_prefix, workspace_id, tier FROM api_keys WHERE revoked = FALSE AND key_prefix = ?",
		prefix,
	)
	if err == nil {
		defer rows.Close()
		for rows.Next() {
			var k APIKey
			if err := rows.Scan(&k.ID, &k.KeyHash, &k.KeyPrefix, &k.WorkspaceID, &k.Tier); err != nil {
				log.Printf("DB error scanning key: %v", err)
				continue
			}

			if err := bcrypt.CompareHashAndPassword([]byte(k.KeyHash), []byte(plaintextKey)); err == nil {
				return &k, nil
			}
		}
	}

	// Backward-compatible fallback for legacy rows without key_prefix.
	fallbackRows, err := s.db.Query("SELECT id, key_hash, key_prefix, workspace_id, tier FROM api_keys WHERE revoked = FALSE")
	if err != nil {
		return nil, err
	}
	defer fallbackRows.Close()

	for fallbackRows.Next() {
		var k APIKey
		if err := fallbackRows.Scan(&k.ID, &k.KeyHash, &k.KeyPrefix, &k.WorkspaceID, &k.Tier); err != nil {
			log.Printf("DB error scanning key: %v", err)
			continue
		}

		if err := bcrypt.CompareHashAndPassword([]byte(k.KeyHash), []byte(plaintextKey)); err == nil {
			return &k, nil
		}
	}

	return nil, fmt.Errorf("invalid api key")
}
