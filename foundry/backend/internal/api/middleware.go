package api

import (
	"context"
	"fmt"
	"net/http"
	"strings"

	"github.com/labwired/foundry-backend/internal/db"
)

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
		ctx := context.WithValue(r.Context(), "api_key", apiKey)
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
		apiKeyVal := r.Context().Value("api_key")
		if apiKeyVal == nil {
			sendError(w, http.StatusUnauthorized, "UNAUTHORIZED", "API Key not found in context.", "Ensure auth middleware is applied before quota middleware.")
			return
		}

		apiKey := apiKeyVal.(*db.APIKey)

		// For MVP, Free tier gets 50 runs per 30 days.
		// Enterprise gets 1,000,000.
		limit := 50
		if apiKey.Tier == "enterprise" {
			limit = 1000000
		}

		count, err := s.store.CountRunsForWorkspace(apiKey.WorkspaceID)
		if err != nil {
			sendError(w, http.StatusInternalServerError, "QUOTA_CHECK_FAILED", "Failed to verify usage quota.", "Try again later.")
			return
		}

		if count >= limit {
			sendError(w, http.StatusTooManyRequests, "QUOTA_EXCEEDED", fmt.Sprintf("Workspace has exceeded its monthly limit of %d runs.", limit), "Upgrade to Enterprise tier for higher volume.")
			return
		}

		next.ServeHTTP(w, r)
	})
}
