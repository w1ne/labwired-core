// EmbedBadge — the "Made with LabWired" corner attribution shown inside an
// embedded (?embed=true) lab. Pinned bottom-right, small and semi-transparent,
// linking back to the full (editable) lab.
//
// Note: the shared `GlobalLogo` is itself an <a>, so we can't nest it inside
// the badge's own link without producing invalid nested-anchor markup. We
// reuse the same logo mark (the SVG from `public/logo.svg` / GlobalLogo) and
// the "LabWired" wordmark inline instead.

/** Current page URL with the `embed` param stripped — deep-links to the editable lab. */
function fullLabUrl(): string {
  const url = new URL(window.location.href);
  url.searchParams.delete('embed');
  return url.toString();
}

export function EmbedBadge() {
  return (
    <a
      href={fullLabUrl()}
      target="_blank"
      rel="noopener noreferrer"
      title="Open the full lab on LabWired"
      className="fixed bottom-2 right-2 z-40 flex items-center gap-1.5 h-6 px-2 rounded-pill bg-black/40 hover:bg-black/60 text-white/70 hover:text-white/90 text-[11px] font-medium tracking-tight backdrop-blur transition-colors duration-150 no-underline shadow-[0_2px_8px_rgba(0,0,0,0.3)]"
    >
      <svg viewBox="0 0 32 32" width="14" height="14" fill="none" aria-hidden="true">
        <path d="M11 7V23H23" stroke="#4d9fff" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round" />
        <circle cx="11" cy="7" r="3" fill="currentColor" />
        <circle cx="23" cy="23" r="3" fill="#4d9fff" />
      </svg>
      <span>Made with LabWired</span>
    </a>
  );
}
