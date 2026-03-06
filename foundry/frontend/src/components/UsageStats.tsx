import { useState, useEffect, useCallback } from 'react';
import { useAuth } from '@clerk/react';
import { apiUrl, getApiKey, setApiKey, STRIPE_PAYMENT_LINK } from '../api';

interface UsageData {
    workspace_id: string;
    tier: string;
    runs_used_this_month: number;
    quota: number;
    runs_remaining: number;
}

interface RunRecord {
    run_id: string;
    status: string;
    assertions_passed: number;
    assertions_total: number;
    created_at: string;
}

const StatusBadge = ({ status }: { status: string }) => {
    const colors: Record<string, string> = {
        pass: 'var(--lw-green)',
        fail: '#ff4444',
        error: '#ff8c00',
        running: '#00d9ff',
        queued: 'var(--lw-gray)',
    };
    return (
        <span style={{
            padding: '3px 10px', borderRadius: '12px',
            fontSize: '0.65rem', fontWeight: 800, fontFamily: 'JetBrains Mono, monospace',
            color: '#fff', background: colors[status] || 'var(--lw-gray)',
            textTransform: 'uppercase',
        }}>
            {status}
        </span>
    );
};

const UsageStats = () => {
    const { getToken } = useAuth();
    const [usage, setUsage] = useState<UsageData | null>(null);
    const [runs, setRuns] = useState<RunRecord[]>([]);
    const [loading, setLoading] = useState(true);
    const [error, setError] = useState('');
    const [apiKeyDisplay, setApiKeyDisplay] = useState(getApiKey());
    const [keyCopied, setKeyCopied] = useState(false);

    const loadData = useCallback(async () => {
        setLoading(true);
        setError('');
        try {
            const token = await getToken();
            const headers: Record<string, string> = { 'Content-Type': 'application/json' };
            if (token) headers['Authorization'] = `Bearer ${token}`;

            const [usageRes, runsRes] = await Promise.all([
                fetch(apiUrl('/v1/usage'), { headers }),
                fetch(apiUrl('/v1/runs'), { headers }),
            ]);

            if (!usageRes.ok) throw new Error(`Usage: ${usageRes.status} ${usageRes.statusText}`);
            if (!runsRes.ok) throw new Error(`Runs: ${runsRes.status} ${runsRes.statusText}`);

            const [usageData, runsData] = await Promise.all([usageRes.json(), runsRes.json()]);
            setUsage(usageData);
            setRuns(Array.isArray(runsData) ? runsData : []);
        } catch (err) {
            setError(err instanceof Error ? err.message : 'Failed to load dashboard data');
        } finally {
            setLoading(false);
        }
    }, [getToken]);

    useEffect(() => { loadData(); }, [loadData]);

    const handleCopyKey = () => {
        navigator.clipboard.writeText(apiKeyDisplay);
        setKeyCopied(true);
        setTimeout(() => setKeyCopied(false), 2000);
    };

    const percentage = usage ? Math.min(100, (usage.runs_used_this_month / usage.quota) * 100) : 0;
    const isLow = percentage > 80;

    return (
        <div style={{ padding: '2rem', flex: 1, maxWidth: '1000px' }}>
            <header style={{ marginBottom: '3rem', display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start' }}>
                <div>
                    <h1 style={{ fontSize: '3rem', color: 'var(--lw-black)' }}>DASHBOARD</h1>
                    <p style={{ color: 'var(--lw-gray)', marginTop: '0.5rem', fontSize: '1.1rem' }}>
                        Your workspace quota, recent runs, and API key.
                    </p>
                </div>
                <button className="secondary" onClick={loadData} disabled={loading} style={{ marginTop: '0.5rem' }}>
                    {loading ? '...' : '↻ REFRESH'}
                </button>
            </header>

            {error && (
                <div className="bento-card" style={{ marginBottom: '2rem', borderColor: '#ff4444' }}>
                    <p style={{ color: '#b42318', fontWeight: 700, margin: 0 }}>{error}</p>
                </div>
            )}

            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(380px, 1fr))', gap: '2rem', marginBottom: '2rem' }}>

                {/* Quota Card */}
                <div className="bento-card" style={{ gridColumn: usage && percentage > 80 ? 'span 2' : 'auto' }}>
                    <h4 style={{ fontSize: '0.7rem', color: 'var(--lw-gray)', marginBottom: '1.5rem', fontWeight: 900, letterSpacing: '0.1em' }}>
                        SIMULATION QUOTA
                    </h4>
                    {loading ? (
                        <p style={{ color: 'var(--lw-gray)' }}>Loading...</p>
                    ) : usage ? (
                        <>
                            <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '12px' }}>
                                <span style={{ fontSize: '0.85rem', color: 'var(--lw-gray)', fontWeight: 600 }}>RUNS CONSUMED</span>
                                <span className="mono" style={{ fontSize: '1.2rem', fontWeight: 900 }}>
                                    {usage.runs_used_this_month} / {usage.quota}
                                </span>
                            </div>
                            <div style={{ height: '12px', width: '100%', background: 'var(--lw-bg-alt)', border: 'var(--lw-border)', borderRadius: '6px', overflow: 'hidden', marginBottom: '1rem' }}>
                                <div style={{
                                    height: '100%', width: `${percentage}%`,
                                    background: isLow ? '#ff4444' : 'var(--lw-pink)',
                                    borderRadius: '6px', transition: 'width 0.3s ease',
                                }} />
                            </div>
                            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                                <p style={{ fontSize: '0.85rem', color: 'var(--lw-gray)', fontWeight: 500, margin: 0 }}>
                                    {usage.runs_remaining} runs remaining · <span style={{ color: 'var(--lw-black)', fontWeight: 800 }}>{usage.tier.toUpperCase()}</span> tier
                                </p>
                                <a href={STRIPE_PAYMENT_LINK} target="_blank" rel="noopener noreferrer">
                                    <button style={{ padding: '8px 16px', fontSize: '0.8rem' }}>
                                        + Buy 1,000 runs
                                    </button>
                                </a>
                            </div>
                        </>
                    ) : (
                        <p style={{ color: 'var(--lw-gray)' }}>No usage data available.</p>
                    )}
                </div>

                {/* API Key Card */}
                <div className="bento-card">
                    <h4 style={{ fontSize: '0.7rem', color: 'var(--lw-gray)', marginBottom: '1.5rem', fontWeight: 900, letterSpacing: '0.1em' }}>
                        API KEY
                    </h4>
                    <div style={{
                        background: '#0d1117', borderRadius: '8px', padding: '12px 16px',
                        fontFamily: 'JetBrains Mono, monospace', fontSize: '0.8rem', color: '#e6edf3',
                        marginBottom: '1rem', wordBreak: 'break-all',
                    }}>
                        {apiKeyDisplay ? `${apiKeyDisplay.slice(0, 16)}...` : 'No key set'}
                    </div>
                    <div style={{ display: 'flex', gap: '0.75rem' }}>
                        <input
                            type="text"
                            placeholder="Paste your API key..."
                            value={apiKeyDisplay}
                            onChange={e => {
                                setApiKeyDisplay(e.target.value);
                                setApiKey(e.target.value);
                            }}
                            style={{
                                flex: 1, background: 'var(--lw-bg-alt)', border: 'var(--lw-border)',
                                borderRadius: '6px', padding: '8px 12px',
                                fontFamily: 'JetBrains Mono, monospace', fontSize: '0.8rem',
                            }}
                        />
                        <button className="secondary" onClick={handleCopyKey} style={{ padding: '8px 14px' }}>
                            {keyCopied ? '✓' : 'COPY'}
                        </button>
                    </div>
                </div>
            </div>

            {/* Recent Runs Table */}
            <div className="bento-card" style={{ marginTop: '1rem' }}>
                <h4 style={{ fontSize: '0.7rem', color: 'var(--lw-gray)', marginBottom: '1.5rem', fontWeight: 900, letterSpacing: '0.1em' }}>
                    RECENT RUNS
                </h4>
                {loading ? (
                    <p style={{ color: 'var(--lw-gray)', fontSize: '0.9rem' }}>Loading...</p>
                ) : runs.length === 0 ? (
                    <p style={{ color: 'var(--lw-gray)', fontStyle: 'italic', fontSize: '0.9rem' }}>
                        No runs yet. Submit your first simulation via the API.
                    </p>
                ) : (
                    <div style={{ overflowX: 'auto' }}>
                        <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: '0.85rem' }}>
                            <thead>
                                <tr style={{ borderBottom: '2px solid var(--lw-black)' }}>
                                    {['Run ID', 'Status', 'Assertions', 'Timestamp', ''].map(h => (
                                        <th key={h} style={{
                                            padding: '0.6rem 0.75rem', textAlign: 'left',
                                            fontFamily: 'Outfit', fontWeight: 800, fontSize: '0.65rem',
                                            letterSpacing: '0.1em', color: 'var(--lw-gray)',
                                        }}>{h}</th>
                                    ))}
                                </tr>
                            </thead>
                            <tbody>
                                {runs.map(run => (
                                    <tr key={run.run_id} style={{ borderBottom: '1px solid #eee' }}>
                                        <td style={{ padding: '0.75rem', fontFamily: 'JetBrains Mono, monospace', fontSize: '0.75rem', color: 'var(--lw-gray)' }}>
                                            {run.run_id.slice(0, 24)}...
                                        </td>
                                        <td style={{ padding: '0.75rem' }}>
                                            <StatusBadge status={run.status} />
                                        </td>
                                        <td style={{ padding: '0.75rem', fontFamily: 'JetBrains Mono, monospace' }}>
                                            {run.assertions_passed}/{run.assertions_total}
                                        </td>
                                        <td style={{ padding: '0.75rem', color: 'var(--lw-gray)', fontSize: '0.8rem' }}>
                                            {new Date(run.created_at).toLocaleString()}
                                        </td>
                                        <td style={{ padding: '0.75rem' }}>
                                            <button className="secondary" style={{ padding: '4px 10px', fontSize: '0.7rem', boxShadow: 'none' }}
                                                onClick={() => window.location.hash = `/runs/${run.run_id}`}>
                                                POLL →
                                            </button>
                                        </td>
                                    </tr>
                                ))}
                            </tbody>
                        </table>
                    </div>
                )}
            </div>
        </div>
    );
};

export default UsageStats;
