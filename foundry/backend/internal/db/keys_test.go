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

func TestCreateKeyForClerkUser_ReusesWorkspaceAndQuota(t *testing.T) {
	store := newTestStore(t)

	firstKey, err := store.CreateKeyForClerkUser("clerk-user-1", "lw_sk_live_first_key_123456", "ws-clerk-1")
	if err != nil {
		t.Fatalf("CreateKeyForClerkUser first failed: %v", err)
	}
	if firstKey.WorkspaceID != "ws-clerk-1" {
		t.Fatalf("unexpected first workspace: got=%s want=ws-clerk-1", firstKey.WorkspaceID)
	}

	if err := store.AddQuotaRuns("ws-clerk-1", 250); err != nil {
		t.Fatalf("AddQuotaRuns failed: %v", err)
	}

	secondKey, err := store.CreateKeyForClerkUser("clerk-user-1", "lw_sk_live_second_key_654321", "ws-ignored")
	if err != nil {
		t.Fatalf("CreateKeyForClerkUser second failed: %v", err)
	}
	if secondKey.WorkspaceID != firstKey.WorkspaceID {
		t.Fatalf("expected reused workspace: got=%s want=%s", secondKey.WorkspaceID, firstKey.WorkspaceID)
	}

	quota, err := store.GetMonthlyQuota(firstKey.WorkspaceID)
	if err != nil {
		t.Fatalf("GetMonthlyQuota failed: %v", err)
	}
	if quota != 1250 {
		t.Fatalf("unexpected reused workspace quota: got=%d want=1250", quota)
	}
}

func TestGetPrimaryWorkspaceForClerkUser_ReturnsFirstActiveWorkspace(t *testing.T) {
	store := newTestStore(t)

	binding, err := store.GetPrimaryWorkspaceForClerkUser("missing-user")
	if err != nil {
		t.Fatalf("GetPrimaryWorkspaceForClerkUser missing failed: %v", err)
	}
	if binding != nil {
		t.Fatalf("expected nil binding for missing user, got %#v", binding)
	}

	first, err := store.CreateKeyForClerkUser("clerk-user-primary", "lw_sk_live_primary_workspace_key_123", "ws-primary")
	if err != nil {
		t.Fatalf("CreateKeyForClerkUser first failed: %v", err)
	}
	if _, err := store.CreateKeyForClerkUser("clerk-user-primary", "lw_sk_live_primary_workspace_key_456", "ws-ignored"); err != nil {
		t.Fatalf("CreateKeyForClerkUser second failed: %v", err)
	}

	binding, err = store.GetPrimaryWorkspaceForClerkUser("clerk-user-primary")
	if err != nil {
		t.Fatalf("GetPrimaryWorkspaceForClerkUser failed: %v", err)
	}
	if binding == nil {
		t.Fatalf("expected workspace binding")
	}
	if binding.WorkspaceID != first.WorkspaceID {
		t.Fatalf("unexpected workspace id: got=%s want=%s", binding.WorkspaceID, first.WorkspaceID)
	}
	if binding.Tier != "builder" {
		t.Fatalf("unexpected tier: got=%s want=builder", binding.Tier)
	}
}
