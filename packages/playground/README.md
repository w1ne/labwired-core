# LabWired Playground

Browser front-end for the LabWired deterministic firmware simulator. Drop MCU
boards and components on a canvas, Run, and watch real firmware execute in WASM.

## Run it locally

```bash
cd packages/playground
cp .env.example .env.local      # enables VITE_DISABLE_AUTH for local dev
npm install
npm run dev                     # → http://localhost:5173
```

Open the URL and click **Run** — no account needed. `.env.example` sets
`VITE_DISABLE_AUTH=true`, which skips the Clerk sign-in gate that the deployed
build uses. (Production never sets that flag, so sign-in stays enforced there.)

`npm run dev` also fetches the bundled demo firmware (`predev` →
`scripts/fetch-demo-firmware.sh`) and the simulator runs fully client-side in
WASM, so no backend is required for the built-in demos.

## Try the BLE demo

1. Press **⌘K** (or click the search bar) and pick **nRF52840 BLE Sensor**.
2. Click **Run**. The floating **Packet Analyzer** (bottom-right) shows the
   sensor's BLE frames de-whitened in real time — an incrementing reading at
   2442 MHz, BLE 1M, address `BE:CAFEBA00`.
3. Add **nRF52840 BLE Collector** to the same canvas to see two radios share
   the simulator's virtual air.

## Scripts

| Command | What it does |
| --- | --- |
| `npm run dev` | Vite dev server (fetches demo firmware first) |
| `npm run build` | Type-check + production build |
| `npm test` | Vitest unit tests |
| `npm run e2e` | Playwright end-to-end tests |
