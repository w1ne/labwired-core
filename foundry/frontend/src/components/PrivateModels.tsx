import { useCallback, useEffect, useState } from 'react';
import { useAuth } from '@clerk/react';
import { apiUrl } from '../api';

type SubmitMode = 'verify-model' | 'verify-system' | 'estimate' | 'synthesize';

interface QuickstartResponse {
    key_prefix: string;
    curl: string;
}

interface SubmitResult {
    run_id?: string;
    poll_url?: string;
    status?: string;
    component_name?: string;
    estimated_cost_runs?: number;
    message?: string;
}

const modeCopy: Record<SubmitMode, { title: string; endpoint: string; button: string; helper: string; sample: string }> = {
    'verify-model': {
        title: 'Verify Model',
        endpoint: '/v1/account/models/verify',
        button: 'Submit model',
        helper: 'Paste model YAML into chip_yaml. This uses your Clerk-owned workspace, not a browser-stored API key.',
        sample: 'device: EXAMPLE\nregisters:\n  - name: CTRL\n    offset: 0x00\n    reset: 0x00',
    },
    'verify-system': {
        title: 'Verify System',
        endpoint: '/v1/account/systems/verify',
        button: 'Submit system',
        helper: 'Paste a system YAML manifest for hosted verification.',
        sample: 'chip: configs/chips/stm32f103.yaml\nperipherals: []',
    },
    estimate: {
        title: 'Estimate Synthesis',
        endpoint: '/v1/account/estimate',
        button: 'Estimate cost',
        helper: 'Use this before submitting a synthesis run.',
        sample: 'Need an I2C accelerometer model with deterministic register behavior.',
    },
    synthesize: {
        title: 'Synthesize',
        endpoint: '/v1/account/synthesize',
        button: 'Submit synthesis',
        helper: 'Creates a hosted synthesis run against your dashboard workspace.',
        sample: 'Need an I2C accelerometer model with deterministic register behavior.',
    },
};

const PrivateModels = () => {
    const { getToken } = useAuth();
    const [mode, setMode] = useState<SubmitMode>('verify-model');
    const [componentName, setComponentName] = useState('ADXL345');
    const [payload, setPayload] = useState(modeCopy['verify-model'].sample);
    const [datasheetUrl, setDatasheetUrl] = useState('');
    const [loading, setLoading] = useState(false);
    const [error, setError] = useState('');
    const [result, setResult] = useState<SubmitResult | null>(null);
    const [quickstart, setQuickstart] = useState<QuickstartResponse | null>(null);

    useEffect(() => {
        setPayload(modeCopy[mode].sample);
        setResult(null);
        setError('');
    }, [mode]);

    const clerkHeaders = useCallback(async (): Promise<Record<string, string>> => {
        const token = await getToken();
        const headers: Record<string, string> = { 'Content-Type': 'application/json' };
        if (token) headers['Authorization'] = `Bearer ${token}`;
        return headers;
    }, [getToken]);

    const loadQuickstart = useCallback(async () => {
        try {
            const headers = await clerkHeaders();
            const response = await fetch(apiUrl('/v1/account/quickstart'), { headers });
            if (!response.ok) {
                return;
            }
            const data = await response.json();
            setQuickstart(data);
        } catch {
            // Keep dashboard usable if this helper fails.
        }
    }, [clerkHeaders]);

    useEffect(() => {
        loadQuickstart();
    }, [loadQuickstart]);

    const handleSubmit = async () => {
        setLoading(true);
        setError('');
        setResult(null);
        try {
            const headers = await clerkHeaders();
            let body: Record<string, string>;
            if (mode === 'verify-model') {
                body = { chip_yaml: payload };
            } else if (mode === 'verify-system') {
                body = { system_yaml: payload };
            } else {
                body = {
                    component_name: componentName,
                    requirements: payload,
                };
                if (datasheetUrl.trim() !== '') {
                    body.datasheet_url = datasheetUrl.trim();
                }
            }
            const response = await fetch(apiUrl(modeCopy[mode].endpoint), {
                method: 'POST',
                headers,
                body: JSON.stringify(body),
            });
            if (!response.ok) {
                const text = await response.text();
                throw new Error(text || `${response.status} ${response.statusText}`);
            }
            const data = await response.json();
            setResult(data);
            if (data?.run_id) {
                window.location.hash = `/runs/${data.run_id}`;
            }
        } catch (err) {
            setError(err instanceof Error ? err.message : 'Submission failed');
        } finally {
            setLoading(false);
        }
    };

    return (
        <div style={{ padding: '2rem', flex: 1, maxWidth: '1100px' }}>
            <header style={{ marginBottom: '2rem' }}>
                <h1 style={{ fontSize: '3rem', color: 'var(--lw-black)' }}>RUN CONSOLE</h1>
                <p style={{ color: 'var(--lw-gray)', marginTop: '0.5rem', maxWidth: '760px', fontSize: '1.1rem' }}>
                    Human dashboard for submitting hosted runs into your Clerk-owned workspace and reviewing the resulting artifacts.
                    API keys remain the primary product interface; this panel is a control-plane convenience.
                </p>
            </header>

            <div style={{ display: 'grid', gridTemplateColumns: 'minmax(0, 2fr) minmax(320px, 1fr)', gap: '2rem', alignItems: 'start' }}>
                <div className="bento-card">
                    <div style={{ display: 'flex', gap: '0.75rem', flexWrap: 'wrap', marginBottom: '1.5rem' }}>
                        {(Object.keys(modeCopy) as SubmitMode[]).map((value) => (
                            <button
                                key={value}
                                className={mode === value ? '' : 'secondary'}
                                onClick={() => setMode(value)}
                                style={{ padding: '0.6rem 1rem', fontSize: '0.8rem' }}
                            >
                                {modeCopy[value].title}
                            </button>
                        ))}
                    </div>

                    <p style={{ color: 'var(--lw-gray)', marginTop: 0, marginBottom: '1.5rem' }}>
                        {modeCopy[mode].helper}
                    </p>

                    {(mode === 'estimate' || mode === 'synthesize') && (
                        <div style={{ marginBottom: '1rem' }}>
                            <label style={{ display: 'block', fontSize: '0.75rem', fontWeight: 800, letterSpacing: '0.08em', color: 'var(--lw-gray)', marginBottom: '0.5rem' }}>
                                COMPONENT NAME
                            </label>
                            <input
                                value={componentName}
                                onChange={(e) => setComponentName(e.target.value)}
                                style={{ width: '100%', padding: '0.8rem 1rem', borderRadius: '8px', border: '1px solid #d0d0d0', fontSize: '0.95rem' }}
                            />
                        </div>
                    )}

                    {(mode === 'estimate' || mode === 'synthesize') && (
                        <div style={{ marginBottom: '1rem' }}>
                            <label style={{ display: 'block', fontSize: '0.75rem', fontWeight: 800, letterSpacing: '0.08em', color: 'var(--lw-gray)', marginBottom: '0.5rem' }}>
                                DATASHEET URL
                            </label>
                            <input
                                value={datasheetUrl}
                                onChange={(e) => setDatasheetUrl(e.target.value)}
                                placeholder="https://vendor.example/datasheet.pdf"
                                style={{ width: '100%', padding: '0.8rem 1rem', borderRadius: '8px', border: '1px solid #d0d0d0', fontSize: '0.95rem' }}
                            />
                        </div>
                    )}

                    <div style={{ marginBottom: '1rem' }}>
                        <label style={{ display: 'block', fontSize: '0.75rem', fontWeight: 800, letterSpacing: '0.08em', color: 'var(--lw-gray)', marginBottom: '0.5rem' }}>
                            {mode === 'verify-model' ? 'MODEL YAML' : mode === 'verify-system' ? 'SYSTEM YAML' : 'REQUIREMENTS'}
                        </label>
                        <textarea
                            value={payload}
                            onChange={(e) => setPayload(e.target.value)}
                            rows={14}
                            style={{ width: '100%', padding: '1rem', borderRadius: '12px', border: '1px solid #d0d0d0', fontSize: '0.92rem', fontFamily: 'JetBrains Mono, monospace', resize: 'vertical' }}
                        />
                    </div>

                    <div style={{ display: 'flex', gap: '0.75rem', alignItems: 'center', flexWrap: 'wrap' }}>
                        <button onClick={handleSubmit} disabled={loading} style={{ padding: '0.85rem 1.4rem' }}>
                            {loading ? 'Submitting...' : modeCopy[mode].button}
                        </button>
                        <button className="secondary" onClick={() => setPayload(modeCopy[mode].sample)} disabled={loading}>
                            Reset sample
                        </button>
                    </div>

                    {error && (
                        <div className="bento-card" style={{ marginTop: '1.5rem', borderColor: '#ff4444' }}>
                            <p style={{ color: '#b42318', fontWeight: 700, margin: 0, whiteSpace: 'pre-wrap' }}>{error}</p>
                        </div>
                    )}
                </div>

                <div style={{ display: 'flex', flexDirection: 'column', gap: '1.5rem' }}>
                    <div className="bento-card">
                        <h3 style={{ marginTop: 0, marginBottom: '1rem' }}>Dashboard Quickstart</h3>
                        <p style={{ color: 'var(--lw-gray)', fontSize: '0.9rem' }}>
                            Use Clerk here for oversight and key management. Use API keys in your real agent or CI client.
                        </p>
                        <pre style={{ margin: 0, padding: '1rem', background: '#0d1117', color: '#e6edf3', borderRadius: '10px', overflowX: 'auto', fontSize: '0.78rem' }}>
                            {quickstart?.curl || 'Create an API key to generate a personalized curl snippet.'}
                        </pre>
                    </div>

                    <div className="bento-card">
                        <h3 style={{ marginTop: 0, marginBottom: '1rem' }}>Last Result</h3>
                        {!result && <p style={{ color: 'var(--lw-gray)', margin: 0 }}>No submission yet.</p>}
                        {result && (
                            <div style={{ display: 'grid', gap: '0.65rem', fontSize: '0.9rem' }}>
                                {result.component_name && <div><strong>Component:</strong> {result.component_name}</div>}
                                {result.estimated_cost_runs !== undefined && <div><strong>Estimated cost:</strong> {result.estimated_cost_runs} runs</div>}
                                {result.run_id && <div><strong>Run:</strong> <code>{result.run_id}</code></div>}
                                {result.status && <div><strong>Status:</strong> {result.status}</div>}
                                {result.message && <div><strong>Message:</strong> {result.message}</div>}
                                {result.poll_url && <div><strong>Poll URL:</strong> <code>{result.poll_url}</code></div>}
                            </div>
                        )}
                    </div>
                </div>
            </div>
        </div>
    );
};

export default PrivateModels;
