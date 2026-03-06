package db

import (
	"fmt"
	"log"

	"github.com/google/uuid"
	"golang.org/x/crypto/bcrypt"
)

type APIKey struct {
	ID           string `json:"id"`
	KeyHash      string `json:"-"`
	KeyPrefix    string `json:"-"`
	WorkspaceID  string `json:"workspace_id"`
	Tier         string `json:"tier"`
	ClerkUserID  string `json:"clerk_user_id,omitempty"`
}

// APIKeyPublic is the safe subset returned to the cabinet (no hash, masked prefix only).
type APIKeyPublic struct {
	ID          string `json:"id"`
	KeyPrefix   string `json:"key_prefix"`
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

// ListKeysForClerkUser returns the public view of all active keys for a Clerk account.
func (s *Store) ListKeysForClerkUser(clerkUserID string) ([]APIKeyPublic, error) {
	rows, err := s.db.Query(
		"SELECT id, key_prefix, workspace_id, tier FROM api_keys WHERE clerk_user_id = ? AND revoked = FALSE ORDER BY rowid ASC",
		clerkUserID,
	)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var keys []APIKeyPublic
	for rows.Next() {
		var k APIKeyPublic
		if err := rows.Scan(&k.ID, &k.KeyPrefix, &k.WorkspaceID, &k.Tier); err != nil {
			return nil, err
		}
		keys = append(keys, k)
	}
	return keys, nil
}

// CreateKeyForClerkUser generates a new workspace + API key for a Clerk account.
// Returns the APIKey with the plaintext key embedded in KeyPrefix for one-time display.
func (s *Store) CreateKeyForClerkUser(clerkUserID, plaintextKey, workspaceID string) (*APIKey, error) {
	hash, err := bcrypt.GenerateFromPassword([]byte(plaintextKey), bcrypt.DefaultCost)
	if err != nil {
		return nil, err
	}
	id := uuid.New().String()
	prefix := keyPrefix(plaintextKey)
	_, err = s.db.Exec(
		"INSERT INTO api_keys (id, key_hash, key_prefix, workspace_id, tier, clerk_user_id) VALUES (?, ?, ?, ?, ?, ?)",
		id, string(hash), prefix, workspaceID, "builder", clerkUserID,
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
		ClerkUserID: clerkUserID,
	}, nil
}

// RevokeKeyForClerkUser revokes a key only if it belongs to the given Clerk account.
func (s *Store) RevokeKeyForClerkUser(clerkUserID, keyID string) (bool, error) {
	res, err := s.db.Exec(
		"UPDATE api_keys SET revoked = TRUE WHERE id = ? AND clerk_user_id = ? AND revoked = FALSE",
		keyID, clerkUserID,
	)
	if err != nil {
		return false, err
	}
	n, err := res.RowsAffected()
	return n > 0, err
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
