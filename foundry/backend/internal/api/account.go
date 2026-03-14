package api

import (
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"net/http"

	"github.com/gorilla/mux"
	"github.com/labwired/foundry-backend/internal/db"
)

// handleAccountUsage returns aggregate quota/usage for the authenticated Clerk user.
func (s *Server) handleAccountUsage(w http.ResponseWriter, r *http.Request) {
	clerkUserID, ok := clerkUserIDFromContext(r.Context())
	if !ok {
		sendError(w, http.StatusUnauthorized, "UNAUTHORIZED", "Clerk user ID not found in context.", "")
		return
	}
	usage, err := s.store.GetAccountUsage(clerkUserID)
	if err != nil {
		sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to fetch usage.", "")
		return
	}
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(usage)
}

// handleAccountRuns returns recent runs across all API keys for the Clerk user.
func (s *Server) handleAccountRuns(w http.ResponseWriter, r *http.Request) {
	clerkUserID, ok := clerkUserIDFromContext(r.Context())
	if !ok {
		sendError(w, http.StatusUnauthorized, "UNAUTHORIZED", "Clerk user ID not found in context.", "")
		return
	}
	runs, err := s.store.ListRunsForClerkUser(clerkUserID)
	if err != nil {
		sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to fetch runs.", "")
		return
	}
	if runs == nil {
		runs = []db.RunRecord{}
	}
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(runs)
}

// handleListAccountKeys returns all non-revoked API keys for the Clerk user.
func (s *Server) handleListAccountKeys(w http.ResponseWriter, r *http.Request) {
	clerkUserID, ok := clerkUserIDFromContext(r.Context())
	if !ok {
		sendError(w, http.StatusUnauthorized, "UNAUTHORIZED", "Clerk user ID not found in context.", "")
		return
	}
	keys, err := s.store.ListKeysForClerkUser(clerkUserID)
	if err != nil {
		sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to fetch API keys.", "")
		return
	}
	if keys == nil {
		keys = []db.APIKeyPublic{}
	}
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(keys)
}

// handleCreateAccountKey generates a new API key for the Clerk user.
// The plaintext key is returned exactly once.
func (s *Server) handleCreateAccountKey(w http.ResponseWriter, r *http.Request) {
	clerkUserID, ok := clerkUserIDFromContext(r.Context())
	if !ok {
		sendError(w, http.StatusUnauthorized, "UNAUTHORIZED", "Clerk user ID not found in context.", "")
		return
	}

	raw := make([]byte, 24)
	if _, err := rand.Read(raw); err != nil {
		sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to generate key.", "")
		return
	}
	plaintextKey := fmt.Sprintf("lw_sk_live_%s", hex.EncodeToString(raw))

	apiKey, err := s.store.CreateKeyForClerkUser(clerkUserID, plaintextKey, "")
	if err != nil {
		sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to store API key.", "")
		return
	}

	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(http.StatusCreated)
	json.NewEncoder(w).Encode(map[string]string{
		"key":          plaintextKey,
		"id":           apiKey.ID,
		"workspace_id": apiKey.WorkspaceID,
	})
}

// handleRevokeAccountKey revokes one of the Clerk user's API keys.
func (s *Server) handleRevokeAccountKey(w http.ResponseWriter, r *http.Request) {
	clerkUserID, ok := clerkUserIDFromContext(r.Context())
	if !ok {
		sendError(w, http.StatusUnauthorized, "UNAUTHORIZED", "Clerk user ID not found in context.", "")
		return
	}
	keyID := mux.Vars(r)["key_id"]
	revoked, err := s.store.RevokeKeyForClerkUser(clerkUserID, keyID)
	if err != nil {
		sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to revoke key.", "")
		return
	}
	if !revoked {
		sendError(w, http.StatusNotFound, "NOT_FOUND", "Key not found or already revoked.", "")
		return
	}
	w.WriteHeader(http.StatusNoContent)
}
