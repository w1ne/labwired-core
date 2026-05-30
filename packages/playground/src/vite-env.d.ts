/// <reference types="vite/client" />

declare const __BUILD_TIME__: number;

interface ImportMetaEnv {
  /** Clerk publishable key (sign-in). Required in production; optional when
   *  VITE_DISABLE_AUTH is set for local dev. */
  readonly VITE_CLERK_PUBLISHABLE_KEY?: string;
  /** Base URL for the LabWired compile/catalog API. */
  readonly VITE_LABWIRED_API_BASE?: string;
  /**
   * Local-dev escape hatch. When `'true'`, the playground skips the Clerk
   * sign-in gate so Run/Step work without an account. Off by default; never
   * set in production. See `.env.example`.
   */
  readonly VITE_DISABLE_AUTH?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
