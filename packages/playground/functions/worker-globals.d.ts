// Minimal ambient declarations for the Cloudflare Workers runtime globals used
// by the edge function, so `functions/` typechecks without pulling in the full
// `@cloudflare/workers-types` dependency. Only what we use is declared.

interface HtmlRewriterElement {
  setAttribute(name: string, value: string): void;
}

interface HtmlRewriterHandlers {
  element(element: HtmlRewriterElement): void;
}

interface HtmlRewriterInstance {
  on(selector: string, handlers: HtmlRewriterHandlers): HtmlRewriterInstance;
  transform(response: Response): Response;
}

declare const HTMLRewriter: {
  new (): HtmlRewriterInstance;
};
