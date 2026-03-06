import { render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App';

let signedIn = true;

vi.mock('@clerk/react', () => ({
    useAuth: () => ({ isSignedIn: signedIn }),
}));

vi.mock('./components/Sidebar', () => ({
    default: () => <div data-testid="sidebar" />,
}));

vi.mock('./components/AssetCatalog', () => ({
    default: () => <div data-testid="asset-catalog" />,
}));

vi.mock('./components/UsageStats', () => ({
    default: () => <div data-testid="usage-stats" />,
}));

vi.mock('./components/LandingPage', () => ({
    default: () => <div data-testid="landing-page" />,
}));

vi.mock('./components/AssetDetail', () => ({
    default: () => <div data-testid="asset-detail" />,
}));

vi.mock('./components/HealthMonitoring', () => ({
    default: () => <div data-testid="health-monitoring" />,
}));

vi.mock('./components/RunDetail', () => ({
    default: () => <div data-testid="run-detail" />,
}));

describe('App routing', () => {
    beforeEach(() => {
        signedIn = true;
        window.location.hash = '';
    });

    it('renders run detail for #/runs/:id when signed in', () => {
        window.location.hash = '#/runs/run-123';
        render(<App />);
        expect(screen.getByTestId('run-detail')).toBeInTheDocument();
    });

    it('renders health monitoring for #/health when signed in', () => {
        window.location.hash = '#/health';
        render(<App />);
        expect(screen.getByTestId('health-monitoring')).toBeInTheDocument();
    });

    it('redirects protected run route to landing when signed out', () => {
        signedIn = false;
        window.location.hash = '#/runs/run-123';
        render(<App />);
        expect(screen.getByTestId('landing-page')).toBeInTheDocument();
    });
});
