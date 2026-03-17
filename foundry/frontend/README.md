# LabWired Foundry Frontend

React frontend for the hosted LabWired Foundry experience.

Current role:
- landing page for hosted verification
- public catalog browsing
- authenticated dashboard for usage, runs, and API keys

Current status:
- beta
- useful for internal and demo flows
- not yet the primary LabWired onboarding path

## Prerequisites

- Node.js 18+
- npm

## Development

```bash
npm ci
npm run build
npm test -- --run
```

To run the dev server:

```bash
npm ci
npm run dev
```

## Environment

- `VITE_API_URL` points the frontend at the Foundry backend.
- `VITE_CLERK_PUBLISHABLE_KEY` enables Clerk-backed dashboard authentication.

If `VITE_CLERK_PUBLISHABLE_KEY` is missing, public pages can still render, but dashboard login will not be available.

## User Workflow Target

For this frontend to be launch-ready, a user must be able to:

1. Sign in.
2. Create an API key.
3. Submit a verification run.
4. Poll run status.
5. Download artifacts.
6. Understand quota and billing state.

Anything short of that should be labeled beta rather than presented as the main product path.
