import { useEffect, useState, useCallback } from 'react';
import { useAuth } from '@clerk/react';
import { apiUrl, authHeaders } from '../api';

interface RunResponse {
    run_id: string;
    status: string;
    assertions_passed: number;
    assertions_total: number;
    created_at: string;
    artifacts?: {
        ir_url?: string;
        vcd_url?: string;
        result_url?: string;
    };
}

interface Props {
    runId: string;
    onBack: () => void;
}

const RunDetail = ({ runId, onBack }: Props) => {
    const { getToken, isSignedIn } = useAuth();
    const [run, setRun] = useState<RunResponse | null>(null);
    const [error, setError] = useState<string>('');
    const [loading, setLoading] = useState(true);

    const loadRun = useCallback(async () => {
        setLoading(true);
        setError('');
        try {
            const clerkToken = isSignedIn ? await getToken() : null;
            const dashboardHeaders: Record<string, string> = {};
            if (clerkToken) dashboardHeaders['Authorization'] = `Bearer ${clerkToken}`;
            const endpoint = clerkToken ? `/v1/account/runs/${runId}` : `/v1/runs/${runId}`;
            const headers = clerkToken ? dashboardHeaders : authHeaders();
            const response = await fetch(apiUrl(endpoint), { headers });
            if (!response.ok) {
                const body = await response.text();
                throw new Error(body || `Run endpoint returned ${response.status}`);
            }
            const data: RunResponse = await response.json();
            setRun(data);
        } catch (err) {
            setError(err instanceof Error ? err.message : 'Failed to fetch run');
        } finally {
            setLoading(false);
        }
    }, [getToken, isSignedIn, runId]);

    useEffect(() => {
        loadRun();
    }, [loadRun]);

    return (
        <div style={{ minHeight: '100vh', background: 'var(--lw-bg)', padding: '2rem 2.5rem', maxWidth: '960px', margin: '0 auto' }}>
            <button className="secondary" onClick={onBack} style={{ marginBottom: '2rem' }}>← Back to dashboard</button>

            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '1rem', flexWrap: 'wrap', gap: '1rem' }}>
                <h1 style={{ fontFamily: 'Outfit', fontSize: '2.5rem', fontWeight: 900, margin: 0 }}>RUN STATUS</h1>
                <button onClick={loadRun}>Refresh</button>
            </div>

            <p style={{ color: 'var(--lw-gray)', marginBottom: '2rem' }}>Run ID: <code>{runId}</code></p>

            {loading && <p style={{ color: 'var(--lw-gray)' }}>Loading run details...</p>}

            {error && !loading && (
                <div className="bento-card">
                    <p style={{ color: '#b42318', fontWeight: 700, marginBottom: '0.5rem' }}>{error}</p>
                    <p style={{ color: 'var(--lw-gray)', margin: 0 }}>
                        {isSignedIn ? 'Check dashboard access, then retry.' : 'Check API key/workspace access, then retry.'}
                    </p>
                </div>
            )}

            {run && !loading && (
                <>
                    <div className="bento-card" style={{ marginBottom: '1rem' }}>
                        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, minmax(140px, 1fr))', gap: '1rem' }}>
                            <div>
                                <div style={{ fontSize: '0.7rem', color: 'var(--lw-gray)', letterSpacing: '0.1em' }}>STATUS</div>
                                <div style={{ fontWeight: 900, textTransform: 'uppercase' }}>{run.status}</div>
                            </div>
                            <div>
                                <div style={{ fontSize: '0.7rem', color: 'var(--lw-gray)', letterSpacing: '0.1em' }}>ASSERTIONS</div>
                                <div style={{ fontWeight: 900 }}>{run.assertions_passed}/{run.assertions_total}</div>
                            </div>
                            <div>
                                <div style={{ fontSize: '0.7rem', color: 'var(--lw-gray)', letterSpacing: '0.1em' }}>CREATED</div>
                                <div style={{ fontWeight: 700 }}>{new Date(run.created_at).toLocaleString()}</div>
                            </div>
                        </div>
                    </div>

                    <div className="bento-card">
                        <h2 style={{ marginTop: 0 }}>Artifacts</h2>
                        {!run.artifacts && <p style={{ color: 'var(--lw-gray)' }}>No artifacts yet. Run may still be processing.</p>}
                        {run.artifacts && (
                            <div style={{ display: 'flex', gap: '0.75rem', flexWrap: 'wrap' }}>
                                {run.artifacts.ir_url && <a href={run.artifacts.ir_url}><button>Download IR</button></a>}
                                {run.artifacts.vcd_url && <a href={run.artifacts.vcd_url}><button className="secondary">Download VCD</button></a>}
                                {run.artifacts.result_url && <a href={run.artifacts.result_url}><button className="secondary">Download Result</button></a>}
                            </div>
                        )}
                    </div>
                </>
            )}
        </div>
    );
};

export default RunDetail;
