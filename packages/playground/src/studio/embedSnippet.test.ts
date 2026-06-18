import { describe, it, expect } from 'vitest';
import { buildEmbedSnippet, EMBED_HEIGHTS } from './embedSnippet';

const URL = 'https://app.labwired.com/?embed=true&share=abc123';

describe('buildEmbedSnippet', () => {
  it('produces the exact responsive iframe markup for the Compact preset', () => {
    expect(buildEmbedSnippet(URL, { height: EMBED_HEIGHTS.Compact })).toBe(
      `<iframe src="https://app.labwired.com/?embed=true&amp;share=abc123" title="LabWired lab" width="100%" height="420" style="border:0;border-radius:8px" loading="lazy" sandbox="allow-scripts allow-same-origin allow-popups"></iframe>`,
    );
  });

  it('produces the exact responsive iframe markup for the Tall preset', () => {
    expect(buildEmbedSnippet(URL, { height: EMBED_HEIGHTS.Tall })).toBe(
      `<iframe src="https://app.labwired.com/?embed=true&amp;share=abc123" title="LabWired lab" width="100%" height="600" style="border:0;border-radius:8px" loading="lazy" sandbox="allow-scripts allow-same-origin allow-popups"></iframe>`,
    );
  });

  it('exposes the documented height presets', () => {
    expect(EMBED_HEIGHTS.Compact).toBe(420);
    expect(EMBED_HEIGHTS.Tall).toBe(600);
  });

  it('includes the security sandbox and lazy-loading attributes', () => {
    const snippet = buildEmbedSnippet(URL, { height: 420 });
    expect(snippet).toContain('sandbox="allow-scripts allow-same-origin allow-popups"');
    expect(snippet).toContain('loading="lazy"');
  });

  it('attribute-escapes the url so a crafted share id cannot break out', () => {
    const evil = 'https://app.labwired.com/?embed=true&share="><script>x</script>';
    const snippet = buildEmbedSnippet(evil, { height: 420 });
    // No raw double-quote, < or > from the url should survive inside the attribute.
    expect(snippet).toContain(
      'src="https://app.labwired.com/?embed=true&amp;share=&quot;&gt;&lt;script&gt;x&lt;/script&gt;"',
    );
    expect(snippet).not.toContain('<script>');
  });
});
