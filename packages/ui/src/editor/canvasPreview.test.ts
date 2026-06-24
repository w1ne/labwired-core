// @vitest-environment jsdom
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { renderCanvasPng } from './canvasPreview';

const SENTINEL = 'data:image/png;base64,SENTINEL';

function makeSvg(): SVGSVGElement {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('viewBox', '0 0 100 50');
  svg.setAttribute('width', '100');
  svg.setAttribute('height', '50');
  svg.classList.add('editor-canvas');
  const rect = document.createElementNS('http://www.w3.org/2000/svg', 'rect');
  rect.setAttribute('width', '100');
  rect.setAttribute('height', '50');
  svg.appendChild(rect);
  return svg as SVGSVGElement;
}

describe('renderCanvasPng', () => {
  let toDataURL: ReturnType<typeof vi.fn>;
  let getContext: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    // jsdom has no real canvas/2D context — stub the rasterization surface.
    toDataURL = vi.fn(() => SENTINEL);
    const ctx = {
      fillStyle: '',
      fillRect: vi.fn(),
      drawImage: vi.fn(),
    };
    getContext = vi.fn(() => ctx);
    vi.spyOn(HTMLCanvasElement.prototype, 'toDataURL').mockImplementation(
      toDataURL as unknown as HTMLCanvasElement['toDataURL'],
    );
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(
      getContext as unknown as HTMLCanvasElement['getContext'],
    );

    // jsdom's <img> never fires load/error on a data: URL; force onload so the
    // helper proceeds to draw. Intrinsic size is 0 in jsdom (we guard for that).
    Object.defineProperty(HTMLImageElement.prototype, 'src', {
      configurable: true,
      set(this: HTMLImageElement) {
        setTimeout(() => this.onload?.(new Event('load')), 0);
      },
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
    // Remove the patched src setter.
    delete (HTMLImageElement.prototype as unknown as Record<string, unknown>).src;
  });

  it('returns the PNG data URL produced by toDataURL', async () => {
    const result = await renderCanvasPng(makeSvg());
    expect(result).toBe(SENTINEL);
    expect(toDataURL).toHaveBeenCalledWith('image/png');
  });

  it('sizes the canvas to the default 1200x630', async () => {
    let captured: { width: number; height: number } | null = null;
    toDataURL.mockImplementation(function (this: HTMLCanvasElement) {
      captured = { width: this.width, height: this.height };
      return SENTINEL;
    });
    await renderCanvasPng(makeSvg());
    expect(captured).toEqual({ width: 1200, height: 630 });
  });

  it('honours custom width/height options', async () => {
    let captured: { width: number; height: number } | null = null;
    toDataURL.mockImplementation(function (this: HTMLCanvasElement) {
      captured = { width: this.width, height: this.height };
      return SENTINEL;
    });
    await renderCanvasPng(makeSvg(), { width: 800, height: 418 });
    expect(captured).toEqual({ width: 800, height: 418 });
  });

  it('returns null when toDataURL throws (tainted canvas)', async () => {
    toDataURL.mockImplementation(() => {
      throw new Error('tainted canvas');
    });
    const result = await renderCanvasPng(makeSvg());
    expect(result).toBeNull();
  });

  it('returns null when the 2D context is unavailable', async () => {
    getContext.mockReturnValue(null);
    const result = await renderCanvasPng(makeSvg());
    expect(result).toBeNull();
  });
});
