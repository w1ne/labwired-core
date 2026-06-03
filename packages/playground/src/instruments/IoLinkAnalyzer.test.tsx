import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render } from '@testing-library/react';
import { IoLinkAnalyzer } from './IoLinkAnalyzer';
import type { SimulatorBridge } from '@labwired/ui';

afterEach(() => cleanup());

describe('IoLinkAnalyzer', () => {
  it('does not poll while stopped', () => {
    const bridge = {
      iolinkTraceSnapshot: vi.fn(() => []),
    } as unknown as SimulatorBridge;

    render(<IoLinkAnalyzer bridge={bridge} running={false} />);

    expect(bridge.iolinkTraceSnapshot).not.toHaveBeenCalled();
  });
});
