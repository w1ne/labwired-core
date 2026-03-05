package api

import (
	"context"
	"fmt"
	"log"
	"net/http"
	"strconv"
	"strings"
	"time"

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
			sendError(w, http.StatusUnauthorized, "UNAUTHORIZED", "Missing or invalid Authorization header.", "Use Authorization: Bearer <api_key>.")
			return
		}

		key := strings.TrimSpace(strings.TrimPrefix(authHeader, "Bearer "))
		if key == "" {
			sendError(w, http.StatusUnauthorized, "UNAUTHORIZED", "API key is empty.", "Use Authorization: Bearer <api_key>.")
			return
		}

		apiKey, err := s.store.ValidateKey(key)
		if err != nil {
			sendError(w, http.StatusUnauthorized, "UNAUTHORIZED", "API key is invalid or revoked.", "Generate a valid key and retry.")
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
		w.Header().Set("Access-Control-Allow-Headers", "Content-Type, Authorization, Idempotency-Key")
		w.Header().Set("Access-Control-Expose-Headers", "X-RateLimit-Limit, X-RateLimit-Remaining, X-RateLimit-Reset")

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

func (s *Server) rateLimitMiddleware(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		apiKey, ok := apiKeyFromContext(r.Context())
		if !ok {
			sendError(w, http.StatusUnauthorized, "UNAUTHORIZED", "API Key not found in context.", "Ensure auth middleware is applied before rate-limit middleware.")
			return
		}

		now := time.Now().UTC()
		keyCount, keyReset, err := s.store.IncrementRateWindow("api_key", apiKey.ID, now, s.rateLimitWindow)
		if err != nil {
			sendError(w, http.StatusInternalServerError, "RATE_LIMIT_CHECK_FAILED", "Failed to enforce request rate limit.", "Retry later.")
			return
		}
		if keyCount > s.rateLimitPerAPIKey {
			s.metrics.RateLimitRejected.Add(1)
			writeRateLimitHeaders(w, s.rateLimitPerAPIKey, 0, keyReset)
			sendError(w, http.StatusTooManyRequests, "RATE_LIMITED", "API key request rate exceeded.", "Retry after the rate-limit window resets.")
			return
		}

		wsCount, wsReset, err := s.store.IncrementRateWindow("workspace", apiKey.WorkspaceID, now, s.rateLimitWindow)
		if err != nil {
			sendError(w, http.StatusInternalServerError, "RATE_LIMIT_CHECK_FAILED", "Failed to enforce request rate limit.", "Retry later.")
			return
		}
		if wsCount > s.rateLimitPerWorkspace {
			s.metrics.RateLimitRejected.Add(1)
			writeRateLimitHeaders(w, s.rateLimitPerWorkspace, 0, wsReset)
			sendError(w, http.StatusTooManyRequests, "RATE_LIMITED", "Workspace request rate exceeded.", "Retry after the rate-limit window resets.")
			return
		}

		remainingByKey := s.rateLimitPerAPIKey - keyCount
		remainingByWorkspace := s.rateLimitPerWorkspace - wsCount
		remaining := remainingByKey
		if remainingByWorkspace < remaining {
			remaining = remainingByWorkspace
		}
		limit := s.rateLimitPerAPIKey
		if s.rateLimitPerWorkspace < limit {
			limit = s.rateLimitPerWorkspace
		}
		writeRateLimitHeaders(w, limit, remaining, wsReset)

		next.ServeHTTP(w, r)
	})
}

func writeRateLimitHeaders(w http.ResponseWriter, limit, remaining int, resetAt time.Time) {
	if remaining < 0 {
		remaining = 0
	}
	w.Header().Set("X-RateLimit-Limit", strconv.Itoa(limit))
	w.Header().Set("X-RateLimit-Remaining", strconv.Itoa(remaining))
	w.Header().Set("X-RateLimit-Reset", strconv.FormatInt(resetAt.Unix(), 10))
}
