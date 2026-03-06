import { useEffect, useState } from 'react';
import { apiUrl } from '../api';

interface HealthPayload {
    status: string;
    timestamp?: string;
    components?: Record<string, unknown>;
}

const HealthMonitoring = () => {
    const [health, setHealth] = useState<HealthPayload | null>(null);
    const [error, setError] = useState<string>('');

    useEffect(() => {
        fetch(apiUrl('/v1/health'))
            .then(async (r) => {
                if (!r.ok) {
                    throw new Error(`Health endpoint returned ${r.status}`);
                }
                return r.json();
            })
            .then(setHealth)
            .catch((err) => setError(err instanceof Error ? err.message : 'Failed to load health'));
    }, []);

    return (
        <div style={{ padding: '2rem', flex: 1, maxWidth: '1000px' }}>
            <header style={{ marginBottom: '2rem' }}>
                <h1 style={{ fontSize: '3rem', color: 'var(--lw-black)' }}>PLATFORM HEALTH</h1>
                <p style={{ color: 'var(--lw-gray)', marginTop: '0.5rem', fontSize: '1.1rem' }}>
                    Live backend service status and runtime metrics.
                </p>
            </header>

            {!health && !error && (
                <p style={{ color: 'var(--lw-gray)' }}>Loading health status...</p>
            )}

            {error && (
                <div className="bento-card">
                    <p style={{ color: '#b42318', fontWeight: 700 }}>{error}</p>
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
