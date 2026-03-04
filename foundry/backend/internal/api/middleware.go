package api

import (
	"context"
	"fmt"
	"log"
	"net/http"
	"strings"

	"github.com/labwired/foundry-backend/internal/db"
)

type contextKey string

const apiKeyContextKey contextKey = "api_key"

func apiKeyFromContext(ctx context.Context) (*db.APIKey, bool) {
	v := ctx.Value(apiKeyContextKey)
	if v == nil {
		return nil, false
	}
	apiKey, ok := v.(*db.APIKey)
	return apiKey, ok
}

func (s *Server) authMiddleware(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		authHeader := r.Header.Get("Authorization")
		if authHeader == "" || !strings.HasPrefix(authHeader, "Bearer ") {
			http.Error(w, "Unauthorized", http.StatusUnauthorized)
			return
		}

		key := strings.TrimPrefix(authHeader, "Bearer ")

		apiKey, err := s.store.ValidateKey(key)
		if err != nil {
			http.Error(w, "Unauthorized", http.StatusUnauthorized)
			return
		}

		// Add API Key info to context
		ctx := context.WithValue(r.Context(), apiKeyContextKey, apiKey)
		next.ServeHTTP(w, r.WithContext(ctx))
	})
}

func (s *Server) loggingMiddleware(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		// Log request details
		next.ServeHTTP(w, r)
	})
}

func (s *Server) corsMiddleware(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Access-Control-Allow-Origin", "*")
		w.Header().Set("Access-Control-Allow-Methods", "GET, POST, PUT, DELETE, OPTIONS")
		w.Header().Set("Access-Control-Allow-Headers", "Content-Type, Authorization")

		if r.Method == "OPTIONS" {
			w.WriteHeader(http.StatusOK)
			return
		}

		next.ServeHTTP(w, r)
	})
}

func (s *Server) quotaMiddleware(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		apiKey, ok := apiKeyFromContext(r.Context())
		if !ok {
			sendError(w, http.StatusUnauthorized, "UNAUTHORIZED", "API Key not found in context.", "Ensure auth middleware is applied before quota middleware.")
			return
		}

		// Read per-workspace quota from DB (set at account creation or topped up via Stripe).
		limit, err := s.store.GetMonthlyQuota(apiKey.WorkspaceID)
		if err != nil {
			// Fall back to sensible tier defaults if DB lookup fails.
			limit = 1000
			if apiKey.Tier == "enterprise" {
				limit = 1000000
			}
		}

		count, err := s.store.CountRunsForWorkspace(apiKey.WorkspaceID)
		if err != nil {
			sendError(w, http.StatusInternalServerError, "QUOTA_CHECK_FAILED", "Failed to verify usage quota.", "Try again later.")
			return
		}

		log.Printf("[QUOTA] Workspace %s: %d/%d used", apiKey.WorkspaceID, count, limit)
		if count >= limit {
			sendError(w, http.StatusTooManyRequests, "QUOTA_EXCEEDED", fmt.Sprintf("Workspace has exceeded its monthly limit of %d runs.", limit), "Purchase more credits or upgrade your tier.")
			return
		}

		next.ServeHTTP(w, r)
	})
}
