package api

import (
	"net/http"

	"github.com/gorilla/mux"
)

// roleLevel maps role names to numeric levels for comparison.
var roleLevel = map[string]int{
	"viewer":    1,
	"developer": 2,
	"admin":     3,
}

// requireRole returns middleware that enforces a minimum role for the
// authenticated Clerk user within the organization specified by {org_id}.
func (s *Server) requireRole(minRole string) func(http.Handler) http.Handler {
	minLevel := roleLevel[minRole]

	return func(next http.Handler) http.Handler {
		return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			clerkUserID, ok := clerkUserIDFromContext(r.Context())
			if !ok {
				sendError(w, http.StatusUnauthorized, "UNAUTHORIZED", "Authentication required.", "")
				return
			}

			orgID := mux.Vars(r)["org_id"]
			if orgID == "" {
				sendError(w, http.StatusBadRequest, "BAD_REQUEST", "Organization ID is required.", "")
				return
			}

			role, err := s.store.GetOrgMemberRole(orgID, clerkUserID)
			if err != nil {
				sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to check permissions.", "")
				return
			}

			if role == "" {
				sendError(w, http.StatusForbidden, "FORBIDDEN", "You are not a member of this organization.", "")
				return
			}

			userLevel := roleLevel[role]
			if userLevel < minLevel {
				sendError(w, http.StatusForbidden, "FORBIDDEN",
					"Insufficient permissions. Required role: "+minRole+", your role: "+role+".", "")
				return
			}

			next.ServeHTTP(w, r)
		})
	}
}
