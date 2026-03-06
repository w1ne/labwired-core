import { useEffect, useState, useCallback } from 'react';
import { useAuth } from '@clerk/react';
import { apiUrl } from '../api';

interface HealthPayload {
    status: string;
    timestamp?: string;
    components?: Record<string, unknown>;
}

const HealthMonitoring = () => {
    const { getToken } = useAuth();
    const [health, setHealth] = useState<HealthPayload | null>(null);
    const [error, setError] = useState('');
    const [loading, setLoading] = useState(true);

    const loadHealth = useCallback(async () => {
        setLoading(true);
        setError('');
        setHealth(null);

        const controller = new AbortController();
        const timeout = setTimeout(() => controller.abort(), 10_000);

        try {
            const token = await getToken();
            const headers: Record<string, string> = {};
            if (token) headers['Authorization'] = `Bearer ${token}`;

            const r = await fetch(apiUrl('/v1/health'), { headers, signal: controller.signal });
            if (!r.ok) throw new Error(`Health endpoint returned ${r.status} ${r.statusText}`);
            setHealth(await r.json());
        } catch (err) {
            if (err instanceof Error && err.name === 'AbortError') {
                setError('Request timed out — backend may be unreachable');
            } else {
                setError(err instanceof Error ? err.message : 'Failed to load health');
            }
        } finally {
            clearTimeout(timeout);
            setLoading(false);
        }
    }, [getToken]);

    useEffect(() => { loadHealth(); }, [loadHealth]);

    return (
        <div style={{ padding: '2rem', flex: 1, maxWidth: '1000px' }}>
            <header style={{ marginBottom: '2rem', display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start' }}>
                <div>
                    <h1 style={{ fontSize: '3rem', color: 'var(--lw-black)' }}>PLATFORM HEALTH</h1>
                    <p style={{ color: 'var(--lw-gray)', marginTop: '0.5rem', fontSize: '1.1rem' }}>
                        Live backend service status and runtime metrics.
                    </p>
                </div>
                <button className="secondary" onClick={loadHealth} disabled={loading} style={{ marginTop: '0.5rem' }}>
                    {loading ? '...' : '↻ REFRESH'}
                </button>
            </header>

            {loading && (
                <p style={{ color: 'var(--lw-gray)' }}>Loading health status...</p>
            )}

            {error && (
                <div className="bento-card">
                    <p style={{ color: '#b42318', fontWeight: 700, margin: 0 }}>{error}</p>
                </div>
            )}

            {health && (
                <div className="bento-card">
                    <div style={{ marginBottom: '1rem', fontWeight: 700 }}>
                        Overall Status: <span style={{ textTransform: 'uppercase' }}>{health.status}</span>
                    </div>
                    <pre style={{
                        margin: 0,
                        padding: '1rem',
                        background: '#0d1117',
                        color: '#e6edf3',
                        borderRadius: '8px',
                        overflowX: 'auto',
                        fontFamily: 'JetBrains Mono, monospace',
                        fontSize: '0.8rem',
                    }}>
                        {JSON.stringify(health, null, 2)}
                    </pre>
                </div>
            )}
        </div>
    );
};

export default HealthMonitoring;
