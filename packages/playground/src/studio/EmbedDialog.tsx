// EmbedDialog — produces copy-paste <iframe> code for embedding a run-only lab,
// with a live preview of exactly what will embed.
//
// Reuse note: the playground has no generic Dialog/Modal primitive — the only
// existing modal (ProjectsModal) is a bespoke component bound to the projects
// API. We follow the same backdrop + stop-propagation idiom and match the
// existing Tailwind styling conventions used across the studio chrome rather
// than introduce a new UI dependency.
import { useEffect, useState } from 'react';
import { generateEmbedUrl, type Diagram, type ShareOptions } from '@labwired/ui';
import { buildEmbedSnippet, EMBED_HEIGHTS, type EmbedHeightPreset } from './embedSnippet';

export interface EmbedDialogProps {
  open: boolean;
  onClose: () => void;
  diagram: Diagram;
  source: string;
  /**
   * Build the auth + preview-image extras for the embed POST (signed-in only;
   * resolves to `{}` for anonymous users / on any failure). Best-effort — a
   * rejection here must not block minting the embed link.
   */
  buildExtras?: () => Promise<ShareOptions>;
  /** Surface failures through the host's toast (mirrors handleShare). */
  onError?: (message: string) => void;
}

export function EmbedDialog({ open, onClose, diagram, source, buildExtras, onError }: EmbedDialogProps) {
  const [embedUrl, setEmbedUrl] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [preset, setPreset] = useState<EmbedHeightPreset>('Compact');
  const [copied, setCopied] = useState(false);

  // Mint (or hash-encode) the embed URL when the dialog opens.
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    setLoading(true);
    setEmbedUrl(null);
    setCopied(false);
    // Best-effort extras (preview PNG + auth token). A failure resolves to {}
    // so the embed link is still minted; it just falls back to the logo card.
    (buildExtras ? buildExtras().catch(() => ({})) : Promise.resolve({}))
      .then((extras) => generateEmbedUrl(diagram, source, extras))
      .then((url) => {
        if (!cancelled) setEmbedUrl(url);
      })
      .catch((err) => {
        if (cancelled) return;
        const message = err instanceof Error ? err.message : String(err);
        onError?.(`Embed failed: ${message}`);
        onClose();
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
    // diagram/source are captured at open-time; re-running on every keystroke
    // would re-mint shares needlessly.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  if (!open) return null;

  const snippet = embedUrl ? buildEmbedSnippet(embedUrl, { height: EMBED_HEIGHTS[preset] }) : '';

  async function handleCopy() {
    if (!snippet) return;
    try {
      await navigator.clipboard.writeText(snippet);
      setCopied(true);
      setTimeout(() => setCopied(false), 1800);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      onError?.(`Copy failed: ${message}`);
    }
  }

  const presetBtn = (p: EmbedHeightPreset) =>
    `h-7 px-3 rounded-pill text-xs font-medium transition-colors duration-micro border-0 outline-none focus-visible:ring-2 focus-visible:ring-accent/50 ${
      preset === p
        ? 'bg-accent text-bg-base'
        : 'bg-white/[0.05] text-fg-secondary hover:bg-white/[0.10] hover:text-fg-primary'
    }`;

  return (
    <div
      className="fixed inset-0 z-[1000] flex items-center justify-center bg-black/55 p-4"
      role="presentation"
      onClick={onClose}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Embed this lab"
        className="flex flex-col w-[min(640px,94vw)] max-h-[88vh] rounded-[10px] bg-bg-elevated border border-white/[0.08] shadow-[0_24px_60px_rgba(0,0,0,0.5)] overflow-hidden"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between px-5 py-3.5 border-b border-white/[0.06]">
          <h2 className="m-0 text-sm font-semibold tracking-[0.02em] text-fg-primary">Embed this lab</h2>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            className="bg-transparent border-0 text-fg-tertiary hover:text-fg-primary text-[22px] leading-none px-1.5 cursor-pointer"
          >
            ×
          </button>
        </div>

        <div className="flex flex-col gap-4 px-5 py-4 overflow-y-auto">
          <p className="m-0 text-xs text-fg-tertiary">
            Drop this read-only, interactive lab into any page. Viewers can run it and press
            buttons, but can't rewire it.
          </p>

          {/* Height presets */}
          <div className="flex items-center gap-2">
            <span className="text-xs text-fg-secondary">Height</span>
            <button type="button" className={presetBtn('Compact')} onClick={() => setPreset('Compact')}>
              Compact
            </button>
            <button type="button" className={presetBtn('Tall')} onClick={() => setPreset('Tall')}>
              Tall
            </button>
          </div>

          {/* Snippet + copy */}
          <div className="flex flex-col gap-2">
            <div className="flex items-center justify-between">
              <span className="text-xs text-fg-secondary">Embed code</span>
              <button
                type="button"
                onClick={handleCopy}
                disabled={loading || !snippet}
                className="h-7 px-3 rounded-pill text-xs font-medium bg-accent text-bg-base hover:bg-accent-hover transition-colors duration-micro disabled:opacity-50 border-0"
              >
                {copied ? 'Copied!' : 'Copy'}
              </button>
            </div>
            <textarea
              readOnly
              spellCheck={false}
              value={loading ? 'Generating embed link…' : snippet}
              onFocusCapture={(e) => e.currentTarget.select()}
              className="w-full h-[84px] resize-none rounded-md bg-bg-base border border-white/[0.08] p-2.5 font-mono text-[11px] leading-relaxed text-fg-secondary outline-none focus-visible:ring-2 focus-visible:ring-accent/50"
            />
          </div>

          {/* Live preview */}
          <div className="flex flex-col gap-2">
            <span className="text-xs text-fg-secondary">Live preview</span>
            <div className="rounded-md overflow-hidden border border-white/[0.08] bg-bg-base">
              {embedUrl ? (
                <iframe
                  src={embedUrl}
                  title="LabWired lab preview"
                  width="100%"
                  height={EMBED_HEIGHTS[preset]}
                  style={{ border: 0, display: 'block' }}
                  sandbox="allow-scripts allow-same-origin allow-popups"
                  loading="lazy"
                />
              ) : (
                <div
                  className="flex items-center justify-center text-xs text-fg-tertiary"
                  style={{ height: EMBED_HEIGHTS[preset] }}
                >
                  {loading ? 'Loading preview…' : 'Preview unavailable'}
                </div>
              )}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
