package db

import (
	"testing"

	"golang.org/x/crypto/bcrypt"
)

func TestValidateKey_UsesPrefixIndexPath(t *testing.T) {
	store := newTestStore(t)
	workspaceID := "ws-prefix"
	plaintext := "lw_sk_live_ws-prefix_1234567890"

	if _, err := store.CreateKey(workspaceID, plaintext); err != nil {
		t.Fatalf("CreateKey failed: %v", err)
	}

	k, err := store.ValidateKey(plaintext)
	if err != nil {
		t.Fatalf("ValidateKey failed: %v", err)
	}
	if k.WorkspaceID != workspaceID {
		t.Fatalf("unexpected workspace id: got=%s want=%s", k.WorkspaceID, workspaceID)
	}
}

func TestBackfillKeyPrefixForPlaintext_EnablesLegacyKeyValidation(t *testing.T) {
	store := newTestStore(t)
	workspaceID := "ws-legacy"
	plaintext := "lw_sk_live_ws-legacy_abcdef"

	hash, err := bcrypt.GenerateFromPassword([]byte(plaintext), bcrypt.DefaultCost)
	if err != nil {
		t.Fatalf("GenerateFromPassword failed: %v", err)
	}

	_, err = store.db.Exec(
		"INSERT INTO api_keys (id, key_hash, key_prefix, workspace_id, tier, monthly_quota, revoked) VALUES (?, ?, '', ?, 'builder', 1000, FALSE)",
		"legacy-id-1", string(hash), workspaceID,
	)
	if err != nil {
		t.Fatalf("insert legacy key failed: %v", err)
	}

	if _, err := store.ValidateKey(plaintext); err == nil {
		t.Fatalf("expected ValidateKey to fail before backfill")
	}

	updated, err := store.BackfillKeyPrefixForPlaintext(plaintext)
	if err != nil {
		t.Fatalf("BackfillKeyPrefixForPlaintext failed: %v", err)
	}
	if !updated {
		t.Fatalf("expected one legacy key to be updated")
	}

	k, err := store.ValidateKey(plaintext)
	if err != nil {
		t.Fatalf("ValidateKey after backfill failed: %v", err)
	}
	if k.WorkspaceID != workspaceID {
		t.Fatalf("unexpected workspace id after backfill: got=%s want=%s", k.WorkspaceID, workspaceID)
	}
}
