import { useState, useEffect, useCallback, useRef } from 'react';
import { useAuth } from '@clerk/react';
import { apiUrl } from '../api';

interface RunResponse {
    run_id: string;
    status: string;
    assertions_passed: number;
    assertions_total: number;
    artifacts?: {
        ir_url?: string;
        vcd_url?: string;
        result_url?: string;
    };
}

interface Props {
    id: string;
    onBack: () => void;
}

const STATUS_COLORS: Record<string, string> = {
    pass: 'var(--lw-green)',
    fail: '#ff4444',
    error: '#ff8c00',
    running: '#00d9ff',
    queued: 'var(--lw-gray)',
};

const AssetDetail = ({ id, onBack }: Props) => {
    const { getToken, isSignedIn, isLoaded } = useAuth();
    const [asset, setAsset] = useState<any>(null);
    const [loading, setLoading] = useState(true);

    // Verification state
    const [runId, setRunId] = useState<string | null>(null);
    const [run, setRun] = useState<RunResponse | null>(null);
    const [verifying, setVerifying] = useState(false);
    const [verifyError, setVerifyError] = useState('');
    const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

    useEffect(() => {
        fetch(apiUrl(`/v1/catalog/${id}`))
            .then(r => r.json())
            .then(data => { setAsset(data); setLoading(false); })
            .catch(() => setLoading(false));
    }, [id]);

    // Poll run status
    const pollRun = useCallback(async (rid: string) => {
        try {
            const token = isSignedIn ? await getToken() : null;
            if (!token) return;
            const resp = await fetch(apiUrl(`/v1/account/runs/${rid}`), {
                headers: { 'Authorization': `Bearer ${token}` },
            });
            if (!resp.ok) return;
            const data: RunResponse = await resp.json();
            setRun(data);
            if (data.status === 'pass' || data.status === 'fail' || data.status === 'error') {
                if (pollRef.current) { clearInterval(pollRef.current); pollRef.current = null; }
            }
        } catch { /* polling failure is non-fatal */ }
    }, [getToken, isSignedIn]);

    useEffect(() => {
        return () => { if (pollRef.current) clearInterval(pollRef.current); };
    }, []);

    const handleRunVerification = useCallback(async () => {
        setVerifying(true);
        setVerifyError('');
        setRun(null);
        setRunId(null);
        try {
            const token = isSignedIn ? await getToken() : null;
            if (!token) { setVerifyError('Sign in to run verifications.'); return; }

            // 1. Fetch the source YAML for this catalog asset
            const sourceResp = await fetch(apiUrl(`/v1/catalog/${id}/source`));
            if (!sourceResp.ok) {
                setVerifyError('No source YAML available for this asset.');
                return;
            }
            const chipYaml = await sourceResp.text();

            // 2. Submit to verification API
            const verifyResp = await fetch(apiUrl('/v1/account/models/verify'), {
                method: 'POST',
                headers: {
                    'Authorization': `Bearer ${token}`,
                    'Content-Type': 'application/json',
                },
                body: JSON.stringify({ chip_yaml: chipYaml }),
            });
            if (!verifyResp.ok) {
                const body = await verifyResp.json().catch(() => ({}));
                setVerifyError(body.message || `Verification failed (${verifyResp.status})`);
                return;
            }
            const { run_id } = await verifyResp.json();
            setRunId(run_id);
            setRun({ run_id, status: 'queued', assertions_passed: 0, assertions_total: 0 });

            // 3. Start polling
            pollRef.current = setInterval(() => pollRun(run_id), 2000);
        } catch (err) {
            setVerifyError(err instanceof Error ? err.message : 'Verification request failed.');
        } finally {
            setVerifying(false);
        }
    }, [getToken, isSignedIn, id, pollRun]);

    if (loading) return (
        <div style={{ padding: '3rem', fontFamily: 'Outfit', fontWeight: 900, fontSize: '1.5rem' }}>
            Loading...
        </div>
    );

    if (!asset) return (
        <div style={{ padding: '3rem' }}>
            <button className="secondary" onClick={onBack}>← Back</button>
            <p style={{ color: 'var(--lw-gray)', marginTop: '1rem' }}>Asset not found.</p>
        </div>
    );

    const passRate = asset.pass_rate ?? 0;
    const registers = asset.registers ?? 0;
    const isVerified = !!asset.verified;
    const isTerminal = run && (run.status === 'pass' || run.status === 'fail' || run.status === 'error');

    return (
        <div style={{ minHeight: '100vh', background: 'var(--lw-bg)', padding: '2rem 2.5rem', maxWidth: '960px', margin: '0 auto' }}>
            <button className="secondary" onClick={onBack} style={{ marginBottom: '2rem' }}>← Back to catalog</button>

            {/* Header: image + title + badge */}
            <div style={{ display: 'flex', gap: '2rem', alignItems: 'flex-start', marginBottom: '2rem', flexWrap: 'wrap' }}>
                {asset.image_url && (
                    <img
                        src={asset.image_url}
                        alt={asset.name || asset.id}
                        style={{ width: 160, height: 160, objectFit: 'contain', borderRadius: 12, background: '#f8f8f8', padding: 12, flexShrink: 0 }}
                        onError={e => { (e.target as HTMLImageElement).style.display = 'none'; }}
                    />
                )}
                <div>
                    <div style={{ display: 'flex', alignItems: 'center', gap: '1.5rem', flexWrap: 'wrap', marginBottom: '1rem' }}>
                        <h1 style={{ fontFamily: 'Outfit', fontSize: '2.5rem', fontWeight: 900 }}>{asset.id || asset.name}</h1>
                        <div style={{
                            background: isVerified ? 'var(--lw-green)' : 'var(--lw-gray)', color: '#000', padding: '6px 16px',
                            borderRadius: '6px', fontWeight: 900, fontSize: '0.85rem',
                            fontFamily: 'JetBrains Mono, monospace',
                            boxShadow: isVerified ? '0 0 12px rgba(39, 201, 63, 0.4)' : 'none',
                            animation: isVerified ? 'pulse 2s infinite' : 'none',
                        }}>
                            {isVerified ? '✓ VERIFIED' : 'MODELED'}
                        </div>
                    </div>
                    <p style={{ color: 'var(--lw-gray)', fontSize: '1.1rem', maxWidth: '600px', lineHeight: 1.7 }}>
                        {asset.description || 'Pre-verified peripheral model.'}
                    </p>
                </div>
            </div>

            {/* Stats */}
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: '1.5rem', marginBottom: '3rem', maxWidth: '600px' }}>
                    {[
                        { label: 'Registers', value: registers },
                    { label: 'Pass Rate', value: isVerified ? `${passRate}%` : 'N/A' },
                    { label: 'Status', value: isVerified ? 'Verified' : 'Modeled' },
                ].map(stat => (
                    <div key={stat.label} className="bento-card" style={{ textAlign: 'center' }}>
                        <div style={{ fontSize: '0.65rem', color: 'var(--lw-gray)', fontWeight: 800, letterSpacing: '0.1em', marginBottom: '0.5rem' }}>
                            {stat.label.toUpperCase()}
                        </div>
                        <div style={{ fontFamily: 'Outfit', fontSize: '1.75rem', fontWeight: 900 }}>{stat.value}</div>
                    </div>
                ))}
            </div>

            {/* Run Verification */}
            <div style={{ marginBottom: '3rem' }}>
                <h2 style={{ fontFamily: 'Outfit', fontSize: '1.5rem', marginBottom: '1rem' }}>Run Simulation</h2>
                <div style={{ display: 'flex', alignItems: 'center', gap: '1rem', flexWrap: 'wrap' }}>
                    {isLoaded && isSignedIn ? (
                        <button
                            onClick={handleRunVerification}
                            disabled={verifying || (!!run && !isTerminal)}
                            style={{ minWidth: 180 }}
                        >
                            {verifying ? 'Submitting...' : run && !isTerminal ? 'Running...' : '▶ Run Verification'}
                        </button>
                    ) : (
                        <p style={{ color: 'var(--lw-gray)', margin: 0 }}>
                            Sign in to run a live verification against this model.
                        </p>
                    )}
                </div>

                {verifyError && (
                    <div className="bento-card" style={{ marginTop: '1rem', borderLeft: '3px solid #ff4444' }}>
                        <p style={{ color: '#b42318', fontWeight: 700, margin: 0 }}>{verifyError}</p>
                    </div>
                )}

                {run && (
                    <div className="bento-card" style={{ marginTop: '1rem' }}>
                        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, minmax(120px, 1fr))', gap: '1rem', marginBottom: run.artifacts ? '1rem' : 0 }}>
                            <div>
                                <div style={{ fontSize: '0.7rem', color: 'var(--lw-gray)', letterSpacing: '0.1em' }}>STATUS</div>
                                <span style={{
                                    display: 'inline-block', marginTop: '0.3rem',
                                    padding: '3px 10px', borderRadius: '12px',
                                    fontSize: '0.65rem', fontWeight: 800, fontFamily: 'JetBrains Mono, monospace',
                                    color: '#fff', background: STATUS_COLORS[run.status] || 'var(--lw-gray)',
                                    textTransform: 'uppercase',
                                }}>
                                    {run.status}
                                </span>
                            </div>
                            <div>
                                <div style={{ fontSize: '0.7rem', color: 'var(--lw-gray)', letterSpacing: '0.1em' }}>ASSERTIONS</div>
                                <div style={{ fontWeight: 900, marginTop: '0.3rem' }}>
                                    {run.assertions_total > 0 ? `${run.assertions_passed}/${run.assertions_total}` : '—'}
                                </div>
                            </div>
                            <div>
                                <div style={{ fontSize: '0.7rem', color: 'var(--lw-gray)', letterSpacing: '0.1em' }}>RUN ID</div>
                                <div style={{ fontFamily: 'JetBrains Mono, monospace', fontSize: '0.75rem', marginTop: '0.3rem', wordBreak: 'break-all' }}>
                                    {runId}
                                </div>
                            </div>
                        </div>
                        {run.artifacts && (
                            <div style={{ display: 'flex', gap: '0.75rem', flexWrap: 'wrap' }}>
                                {run.artifacts.ir_url && <a href={run.artifacts.ir_url}><button>Download IR</button></a>}
                                {run.artifacts.vcd_url && <a href={run.artifacts.vcd_url}><button className="secondary">Download VCD</button></a>}
                                {run.artifacts.result_url && <a href={run.artifacts.result_url}><button className="secondary">Download Result</button></a>}
                            </div>
                        )}
                    </div>
                )}
            </div>

            {/* Register map table */}
            {asset.register_map && asset.register_map.length > 0 && (
                <div style={{ marginBottom: '3rem' }}>
                    <h2 style={{ fontFamily: 'Outfit', fontSize: '1.5rem', marginBottom: '1rem' }}>Register Map</h2>
                    <div style={{ overflowX: 'auto' }}>
                        <table style={{ width: '100%', borderCollapse: 'collapse', fontFamily: 'JetBrains Mono, monospace', fontSize: '0.85rem' }}>
                            <thead>
                                <tr style={{ borderBottom: '2px solid #000' }}>
                                    {['Offset', 'Name', 'Reset', 'Access', 'Description'].map(h => (
                                        <th key={h} style={{ padding: '0.75rem', textAlign: 'left', fontFamily: 'Outfit', fontWeight: 800, fontSize: '0.7rem', letterSpacing: '0.1em' }}>{h.toUpperCase()}</th>
                                    ))}
                                </tr>
                            </thead>
                            <tbody>
                                {asset.register_map.map((reg: any, i: number) => (
                                    <tr key={i} style={{ borderBottom: '1px solid #eee' }}>
                                        <td style={{ padding: '0.75rem', color: 'var(--lw-pink)', fontWeight: 600 }}>{reg.offset}</td>
                                        <td style={{ padding: '0.75rem', fontWeight: 700 }}>{reg.name}</td>
                                        <td style={{ padding: '0.75rem', color: 'var(--lw-gray)' }}>{reg.reset_value ?? '—'}</td>
                                        <td style={{ padding: '0.75rem', color: 'var(--lw-gray)' }}>{reg.access ?? '—'}</td>
                                        <td style={{ padding: '0.75rem', color: 'var(--lw-gray)', fontFamily: 'Inter, sans-serif' }}>{reg.description ?? ''}</td>
                                    </tr>
                                ))}
                            </tbody>
                        </table>
                    </div>
                </div>
            )}

            {/* Downloads */}
            <h2 style={{ fontFamily: 'Outfit', fontSize: '1.5rem', marginBottom: '1rem' }}>Artifacts</h2>
            <div style={{ color: 'var(--lw-gray)', marginTop: '-0.25rem', marginBottom: '1rem', fontSize: '0.9rem', display: 'flex', flexDirection: 'column', gap: '0.4rem' }}>
                <span>Source: {asset.source_type || 'unknown'} {asset.source_ref ? `(${asset.source_ref})` : ''}</span>
                <div style={{ display: 'flex', gap: '1rem', flexWrap: 'wrap' }}>
                    {asset.source_url && (
                        <a href={asset.source_url} target="_blank" rel="noreferrer" style={{ color: 'var(--lw-blue)' }}>
                            Source link
                        </a>
                    )}
                    {asset.official_url && (
                        <a href={asset.official_url} target="_blank" rel="noreferrer" style={{ color: 'var(--lw-blue)' }}>
                            Official board page
                        </a>
                    )}
                    {asset.validation_url && (
                        <a href={asset.validation_url} target="_blank" rel="noreferrer" style={{ color: 'var(--lw-blue)' }}>
                            Latest validation artifacts
                        </a>
                    )}
                </div>
            </div>
            <div style={{ display: 'flex', gap: '1rem', flexWrap: 'wrap' }}>
                {asset.ir_url && (
                    <a href={asset.ir_url} download>
                        <button>⬇ Strict IR (.json)</button>
                    </a>
                )}
                {asset.vcd_url && (
                    <a href={asset.vcd_url} download>
                        <button className="secondary">⬇ Proof VCD (.vcd)</button>
                    </a>
                )}
                {asset.result_url && (
                    <a href={asset.result_url} download>
                        <button className="secondary">⬇ Result (.json)</button>
                    </a>
                )}
            </div>
        </div>
    );
};

export default AssetDetail;
