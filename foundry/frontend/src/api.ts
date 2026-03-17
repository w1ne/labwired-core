// Central API config — reads from env in dev, defaults to same-origin in prod.
const API_BASE = import.meta.env.VITE_API_URL || '';

export function apiUrl(path: string): string {
    return `${API_BASE}${path}`;
}

export function getApiKey(): string {
    return localStorage.getItem('lw_api_key') || '';
}

export function setApiKey(key: string): void {
    localStorage.setItem('lw_api_key', key);
}

export function authHeaders(): Record<string, string> {
    const key = getApiKey();
    return key ? { 'Authorization': `Bearer ${key}`, 'Content-Type': 'application/json' } : {};
}

// Stripe payment link — replace with your real link before going live.
export const STRIPE_PAYMENT_LINK = 'https://buy.stripe.com/test_placeholder';

export function hasLiveStripePaymentLink(): boolean {
    return !STRIPE_PAYMENT_LINK.includes('test_placeholder');
}
