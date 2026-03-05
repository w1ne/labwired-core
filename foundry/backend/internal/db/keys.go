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

	// Fast path only: narrow candidates via indexed prefix.
	rows, err := s.db.Query(
		"SELECT id, key_hash, key_prefix, workspace_id, tier FROM api_keys WHERE revoked = FALSE AND key_prefix = ?",
		prefix,
	)
	if err != nil {
		return nil, err
	}
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

	return nil, fmt.Errorf("invalid api key")
}

// BackfillKeyPrefixForPlaintext updates one legacy key row that is missing key_prefix.
// It is intended for explicit migration workflows, not request-path validation.
func (s *Store) BackfillKeyPrefixForPlaintext(plaintextKey string) (bool, error) {
	rows, err := s.db.Query("SELECT id, key_hash FROM api_keys WHERE revoked = FALSE AND key_prefix = ''")
	if err != nil {
		return false, err
	}
	defer rows.Close()

	for rows.Next() {
		var id, hash string
		if err := rows.Scan(&id, &hash); err != nil {
			return false, err
		}
		if err := bcrypt.CompareHashAndPassword([]byte(hash), []byte(plaintextKey)); err != nil {
			continue
		}

		_, err := s.db.Exec("UPDATE api_keys SET key_prefix = ? WHERE id = ?", keyPrefix(plaintextKey), id)
		if err != nil {
			return false, err
		}
		return true, nil
	}

	return false, nil
}

// BackfillKeyPrefixes attempts backfill for each provided plaintext key and returns rows updated.
func (s *Store) BackfillKeyPrefixes(plaintextKeys []string) (int, error) {
	updated := 0
	for _, plaintext := range plaintextKeys {
		ok, err := s.BackfillKeyPrefixForPlaintext(plaintext)
		if err != nil {
			return updated, err
		}
		if ok {
			updated++
		}
	}
	return updated, nil
}
