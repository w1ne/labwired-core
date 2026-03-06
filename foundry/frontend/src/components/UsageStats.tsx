import { useState, useEffect, useCallback } from 'react';
import { useAuth } from '@clerk/react';
import { apiUrl, STRIPE_PAYMENT_LINK } from '../api';

interface AccountUsage {
    clerk_user_id: string;
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

interface APIKeyPublic {
    id: string;
    key_prefix: string;
    workspace_id: string;
    tier: string;
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
    const [usage, setUsage] = useState<AccountUsage | null>(null);
    const [runs, setRuns] = useState<RunRecord[]>([]);
    const [keys, setKeys] = useState<APIKeyPublic[]>([]);
    const [loading, setLoading] = useState(true);
    const [error, setError] = useState('');
    const [newKey, setNewKey] = useState('');
    const [copiedKey, setCopiedKey] = useState('');
    const [creatingKey, setCreatingKey] = useState(false);

    const clerkHeaders = useCallback(async (): Promise<Record<string, string>> => {
        const token = await getToken();
        const h: Record<string, string> = { 'Content-Type': 'application/json' };
        if (token) h['Authorization'] = `Bearer ${token}`;
        return h;
    }, [getToken]);

    const loadData = useCallback(async () => {
        setLoading(true);
        setError('');
        try {
            const headers = await clerkHeaders();
            const [usageRes, runsRes, keysRes] = await Promise.all([
                fetch(apiUrl('/v1/account/usage'), { headers }),
                fetch(apiUrl('/v1/account/runs'), { headers }),
                fetch(apiUrl('/v1/account/keys'), { headers }),
            ]);

            if (!usageRes.ok) throw new Error(`Usage: ${usageRes.status} ${usageRes.statusText}`);
            if (!runsRes.ok) throw new Error(`Runs: ${runsRes.status} ${runsRes.statusText}`);
            if (!keysRes.ok) throw new Error(`Keys: ${keysRes.status} ${keysRes.statusText}`);

            const [usageData, runsData, keysData] = await Promise.all([
                usageRes.json(), runsRes.json(), keysRes.json(),
            ]);
            setUsage(usageData);
            setRuns(Array.isArray(runsData) ? runsData : []);
            setKeys(Array.isArray(keysData) ? keysData : []);
        } catch (err) {
            setError(err instanceof Error ? err.message : 'Failed to load dashboard data');
        } finally {
            setLoading(false);
        }
    }, [clerkHeaders]);

    useEffect(() => { loadData(); }, [loadData]);

    const handleCreateKey = async () => {
        setCreatingKey(true);
        try {
            const headers = await clerkHeaders();
            const res = await fetch(apiUrl('/v1/account/keys'), { method: 'POST', headers });
            if (!res.ok) throw new Error(`${res.status} ${res.statusText}`);
            const data = await res.json();
            setNewKey(data.key);
            await loadData();
        } catch (err) {
            setError(err instanceof Error ? err.message : 'Failed to create API key');
        } finally {
            setCreatingKey(false);
        }
    };

    const handleRevokeKey = async (keyId: string) => {
        try {
            const headers = await clerkHeaders();
            const res = await fetch(apiUrl(`/v1/account/keys/${keyId}`), { method: 'DELETE', headers });
            if (!res.ok) throw new Error(`${res.status} ${res.statusText}`);
            await loadData();
        } catch (err) {
            setError(err instanceof Error ? err.message : 'Failed to revoke key');
        }
    };

    const handleCopy = (text: string, id: string) => {
        navigator.clipboard.writeText(text);
        setCopiedKey(id);
        setTimeout(() => setCopiedKey(''), 2000);
    };

    const percentage = usage ? Math.min(100, (usage.runs_used_this_month / usage.quota) * 100) : 0;
    const isLow = percentage > 80;

    return (
        <div style={{ padding: '2rem', flex: 1, maxWidth: '1000px' }}>
            <header style={{ marginBottom: '3rem', display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start' }}>
                <div>
                    <h1 style={{ fontSize: '3rem', color: 'var(--lw-black)' }}>DASHBOARD</h1>
                    <p style={{ color: 'var(--lw-gray)', marginTop: '0.5rem', fontSize: '1.1rem' }}>
                        Your workspace quota, recent runs, and API keys.
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

            {/* New key banner — shown once after creation */}
            {newKey && (
                <div className="bento-card" style={{ marginBottom: '2rem', borderColor: 'var(--lw-green)' }}>
                    <p style={{ fontSize: '0.75rem', fontWeight: 800, color: 'var(--lw-green)', marginBottom: '0.75rem' }}>
                        NEW API KEY — copy it now, it won't be shown again
                    </p>
                    <div style={{ display: 'flex', gap: '0.75rem', alignItems: 'center' }}>
                        <code style={{
                            flex: 1, background: '#0d1117', color: '#39ff14',
                            padding: '10px 14px', borderRadius: '6px',
                            fontFamily: 'JetBrains Mono, monospace', fontSize: '0.85rem', wordBreak: 'break-all',
                        }}>{newKey}</code>
                        <button onClick={() => handleCopy(newKey, 'new')}>
                            {copiedKey === 'new' ? '✓ COPIED' : 'COPY'}
                        </button>
                        <button className="secondary" onClick={() => setNewKey('')}>DISMISS</button>
                    </div>
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
                                    <button style={{ padding: '8px 16px', fontSize: '0.8rem' }}>+ Buy 1,000 runs</button>
                                </a>
                            </div>
                        </>
                    ) : (
                        <p style={{ color: 'var(--lw-gray)' }}>No usage data available.</p>
                    )}
                </div>

                {/* API Keys Card */}
                <div className="bento-card">
                    <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '1.5rem' }}>
                        <h4 style={{ fontSize: '0.7rem', color: 'var(--lw-gray)', fontWeight: 900, letterSpacing: '0.1em', margin: 0 }}>
                            API KEYS
                        </h4>
                        <button onClick={handleCreateKey} disabled={creatingKey} style={{ padding: '6px 14px', fontSize: '0.75rem' }}>
                            {creatingKey ? '...' : '+ NEW KEY'}
                        </button>
                    </div>
                    {loading ? (
                        <p style={{ color: 'var(--lw-gray)', fontSize: '0.9rem' }}>Loading...</p>
                    ) : keys.length === 0 ? (
                        <p style={{ color: 'var(--lw-gray)', fontSize: '0.9rem', fontStyle: 'italic' }}>
                            No API keys yet. Create one to start making API calls.
                        </p>
                    ) : (
                        <div style={{ display: 'flex', flexDirection: 'column', gap: '0.5rem' }}>
                            {keys.map(k => (
                                <div key={k.id} style={{
                                    display: 'flex', alignItems: 'center', gap: '0.5rem',
                                    background: 'var(--lw-bg-alt)', borderRadius: '6px', padding: '8px 12px',
                                }}>
                                    <code style={{ flex: 1, fontFamily: 'JetBrains Mono, monospace', fontSize: '0.8rem', color: 'var(--lw-gray)' }}>
                                        {k.key_prefix}...
                                    </code>
                                    <span style={{ fontSize: '0.65rem', fontWeight: 800, color: 'var(--lw-pink)' }}>{k.tier.toUpperCase()}</span>
                                    <button className="secondary" onClick={() => handleCopy(k.key_prefix, k.id)} style={{ padding: '4px 10px', fontSize: '0.65rem', boxShadow: 'none' }}>
                                        {copiedKey === k.id ? '✓' : 'COPY PREFIX'}
                                    </button>
                                    <button className="secondary" onClick={() => handleRevokeKey(k.id)} style={{ padding: '4px 10px', fontSize: '0.65rem', boxShadow: 'none', color: '#ff4444', borderColor: '#ff4444' }}>
                                        REVOKE
                                    </button>
                                </div>
                            ))}
                        </div>
                    )}
                </div>
            </div>

            {/* Recent Runs */}
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
                                        <td style={{ padding: '0.75rem' }}><StatusBadge status={run.status} /></td>
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
