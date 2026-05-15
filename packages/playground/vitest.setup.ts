import '@testing-library/jest-dom';

// cmdk uses ResizeObserver internally; jsdom doesn't ship it
if (typeof globalThis.ResizeObserver === 'undefined') {
  globalThis.ResizeObserver = class ResizeObserver {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

// cmdk calls scrollIntoView on list items; jsdom doesn't implement it
if (typeof Element.prototype.scrollIntoView === 'undefined') {
  Element.prototype.scrollIntoView = () => {};
}
