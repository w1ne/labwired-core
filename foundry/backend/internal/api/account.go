package api

import (
	"bytes"
	"context"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"net/http"
	"os"
	"path/filepath"

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

func (s *Server) accountAPIKeyFromRequest(r *http.Request) (*db.APIKey, error) {
	clerkUserID, ok := clerkUserIDFromContext(r.Context())
	if !ok {
		return nil, fmt.Errorf("clerk user id missing")
	}
	binding, err := s.store.GetPrimaryWorkspaceForClerkUser(clerkUserID)
	if err != nil {
		return nil, err
	}
	if binding == nil {
		return nil, nil
	}
	return &db.APIKey{
		WorkspaceID: binding.WorkspaceID,
		Tier:        binding.Tier,
		ClerkUserID: clerkUserID,
	}, nil
}

func (s *Server) withAccountWorkspace(w http.ResponseWriter, r *http.Request, next func(http.ResponseWriter, *http.Request)) {
	apiKey, err := s.accountAPIKeyFromRequest(r)
	if err != nil {
		sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to resolve dashboard workspace.", "")
		return
	}
	if apiKey == nil {
		sendError(w, http.StatusBadRequest, "NO_WORKSPACE", "No workspace is available for this account.", "Create an API key first to initialize a workspace.")
		return
	}
	ctx := context.WithValue(r.Context(), apiKeyContextKey, apiKey)
	next(w, r.WithContext(ctx))
}

func (s *Server) handleAccountEstimate(w http.ResponseWriter, r *http.Request) {
	s.withAccountWorkspace(w, r, s.handleEstimate)
}

func (s *Server) handleAccountSynthesize(w http.ResponseWriter, r *http.Request) {
	s.withAccountWorkspace(w, r, s.handleSynthesize)
}

func (s *Server) handleAccountVerifyModel(w http.ResponseWriter, r *http.Request) {
	s.withAccountWorkspace(w, r, s.handleVerifyModel)
}

func (s *Server) handleAccountVerifySystem(w http.ResponseWriter, r *http.Request) {
	s.withAccountWorkspace(w, r, s.handleVerifySystem)
}

// handleAccountRun returns a single run owned by the authenticated Clerk user.
func (s *Server) handleAccountRun(w http.ResponseWriter, r *http.Request) {
	clerkUserID, ok := clerkUserIDFromContext(r.Context())
	if !ok {
		sendError(w, http.StatusUnauthorized, "UNAUTHORIZED", "Clerk user ID not found in context.", "")
		return
	}
	runID := mux.Vars(r)["run_id"]
	record, err := s.store.GetRunForClerkUser(runID, clerkUserID)
	if err != nil {
		sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to fetch run status.", "")
		return
	}
	if record == nil {
		sendError(w, http.StatusNotFound, "RUN_NOT_FOUND", "No run found with that ID.", "")
		return
	}

	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(s.runResponse(record, "/v1/account/runs/"+record.RunID+"/artifacts"))
}

// handleAccountRunArtifact returns a run artifact owned by the authenticated Clerk user.
func (s *Server) handleAccountRunArtifact(w http.ResponseWriter, r *http.Request) {
	clerkUserID, ok := clerkUserIDFromContext(r.Context())
	if !ok {
		sendError(w, http.StatusUnauthorized, "UNAUTHORIZED", "Clerk user ID not found in context.", "")
		return
	}
	runID := mux.Vars(r)["run_id"]
	record, err := s.store.GetRunForClerkUser(runID, clerkUserID)
	if err != nil {
		sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to fetch run status.", "")
		return
	}
	if record == nil {
		sendError(w, http.StatusNotFound, "RUN_NOT_FOUND", "No run found with that ID.", "")
		return
	}
	s.serveRunArtifact(w, r, record)
}

func (s *Server) serveRunArtifact(w http.ResponseWriter, r *http.Request, record *db.RunRecord) {
	if record.ArtifactsPath == "" {
		sendError(w, http.StatusNotFound, "ARTIFACT_NOT_FOUND", "No artifacts available for this run.", "")
		return
	}

	name := mux.Vars(r)["file"]
	allowed := map[string]struct{}{
		"output.json": {},
		"proof.vcd":   {},
		"result.json": {},
		"error.log":   {},
	}
	if _, ok := allowed[name]; !ok {
		sendError(w, http.StatusNotFound, "ARTIFACT_NOT_FOUND", "Requested artifact is not available.", "")
		return
	}

	artifactPath := filepath.Join(record.ArtifactsPath, name)
	if _, err := os.Stat(artifactPath); err != nil {
		sendError(w, http.StatusNotFound, "ARTIFACT_NOT_FOUND", "Requested artifact was not found.", "")
		return
	}

	http.ServeFile(w, r, artifactPath)
}

// handleAccountUsageBreakdown returns usage grouped by operation type.
func (s *Server) handleAccountUsageBreakdown(w http.ResponseWriter, r *http.Request) {
	clerkUserID, ok := clerkUserIDFromContext(r.Context())
	if !ok {
		sendError(w, http.StatusUnauthorized, "UNAUTHORIZED", "Clerk user ID not found in context.", "")
		return
	}
	breakdown, err := s.store.GetUsageBreakdownForClerkUser(clerkUserID)
	if err != nil {
		sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to fetch usage breakdown.", "")
		return
	}
	if breakdown == nil {
		breakdown = []db.UsageBreakdown{}
	}
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(breakdown)
}

// handleCreateOrg creates a new organization with the caller as admin.
func (s *Server) handleCreateOrg(w http.ResponseWriter, r *http.Request) {
	clerkUserID, ok := clerkUserIDFromContext(r.Context())
	if !ok {
		sendError(w, http.StatusUnauthorized, "UNAUTHORIZED", "Clerk user ID not found in context.", "")
		return
	}

	var body struct {
		Name string `json:"name"`
	}
	if err := json.NewDecoder(r.Body).Decode(&body); err != nil || body.Name == "" {
		sendError(w, http.StatusBadRequest, "BAD_REQUEST", "Request body must contain 'name'.", "")
		return
	}

	raw := make([]byte, 16)
	if _, err := rand.Read(raw); err != nil {
		sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to generate org ID.", "")
		return
	}
	orgID := hex.EncodeToString(raw)

	if err := s.store.CreateOrg(orgID, body.Name, clerkUserID); err != nil {
		sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to create organization.", "")
		return
	}

	_ = s.store.RecordAuditEvent(orgID, clerkUserID, "create_org", "organization", orgID)

	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(http.StatusCreated)
	json.NewEncoder(w).Encode(map[string]string{
		"id":   orgID,
		"name": body.Name,
	})
}

// handleListOrgs returns all organizations the caller belongs to.
func (s *Server) handleListOrgs(w http.ResponseWriter, r *http.Request) {
	clerkUserID, ok := clerkUserIDFromContext(r.Context())
	if !ok {
		sendError(w, http.StatusUnauthorized, "UNAUTHORIZED", "Clerk user ID not found in context.", "")
		return
	}
	orgs, err := s.store.ListOrgsForUser(clerkUserID)
	if err != nil {
		sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to list organizations.", "")
		return
	}
	if orgs == nil {
		orgs = []db.OrgRecord{}
	}
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(orgs)
}

// handleAddOrgMember adds a member to an organization (admin only, enforced by RBAC middleware).
func (s *Server) handleAddOrgMember(w http.ResponseWriter, r *http.Request) {
	clerkUserID, ok := clerkUserIDFromContext(r.Context())
	if !ok {
		sendError(w, http.StatusUnauthorized, "UNAUTHORIZED", "Clerk user ID not found in context.", "")
		return
	}
	orgID := mux.Vars(r)["org_id"]

	var body struct {
		ClerkUserID string `json:"clerk_user_id"`
		Role        string `json:"role"`
	}
	if err := json.NewDecoder(r.Body).Decode(&body); err != nil || body.ClerkUserID == "" {
		sendError(w, http.StatusBadRequest, "BAD_REQUEST", "Request body must contain 'clerk_user_id'.", "")
		return
	}
	if body.Role == "" {
		body.Role = "developer"
	}
	if _, valid := roleLevel[body.Role]; !valid {
		sendError(w, http.StatusBadRequest, "BAD_REQUEST", "Invalid role. Must be: admin, developer, or viewer.", "")
		return
	}

	if err := s.store.AddOrgMember(orgID, body.ClerkUserID, body.Role); err != nil {
		sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to add member.", "")
		return
	}

	_ = s.store.RecordAuditEvent(orgID, clerkUserID, "add_member", "org_member", body.ClerkUserID)

	w.WriteHeader(http.StatusCreated)
	json.NewEncoder(w).Encode(map[string]string{"status": "added"})
}

// handleListOrgMembers returns all members of an organization (viewer+ access via RBAC).
func (s *Server) handleListOrgMembers(w http.ResponseWriter, r *http.Request) {
	orgID := mux.Vars(r)["org_id"]
	members, err := s.store.ListOrgMembers(orgID)
	if err != nil {
		sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to list members.", "")
		return
	}
	if members == nil {
		members = []db.OrgMember{}
	}
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(members)
}

// handleOrgAuditLog returns audit log entries for an organization (admin only via RBAC).
func (s *Server) handleOrgAuditLog(w http.ResponseWriter, r *http.Request) {
	orgID := mux.Vars(r)["org_id"]
	entries, err := s.store.ListAuditLog(orgID, 200)
	if err != nil {
		sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to fetch audit log.", "")
		return
	}
	if entries == nil {
		entries = []db.AuditEntry{}
	}
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(entries)
}

func (s *Server) handleAccountQuickstart(w http.ResponseWriter, r *http.Request) {
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
	keyPrefix := ""
	if len(keys) > 0 {
		keyPrefix = keys[0].KeyPrefix
	}

	var snippet bytes.Buffer
	fmt.Fprintf(&snippet, "curl -X POST https://<your-foundry-host>/v1/models/verify \\\n")
	fmt.Fprintf(&snippet, "  -H \"Authorization: Bearer %s<full_api_key>\" \\\n", keyPrefix)
	fmt.Fprintf(&snippet, "  -H \"Content-Type: application/json\" \\\n")
	fmt.Fprintf(&snippet, "  -d '{\"chip_yaml\":\"device: EXAMPLE\\nregisters: ...\"}'\n")

	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(map[string]string{
		"key_prefix": keyPrefix,
		"curl":       snippet.String(),
	})
}
