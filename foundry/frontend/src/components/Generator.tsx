import { useState, useEffect } from 'react';

const Generator = () => {
    const [prompt, setPrompt] = useState('');
    const [isGenerating, setIsGenerating] = useState(false);
    const [runId, setRunId] = useState<string | null>(null);
    const [status, setStatus] = useState<any>(null);

    const handleGenerate = async () => {
        setIsGenerating(true);
        setStatus('QUEUING');

        try {
            const response = await fetch('http://localhost:8080/v1/twins/simulate', {
                method: 'POST',
                headers: {
                    'Content-Type': 'application/json',
                    'Authorization': 'Bearer local-dev-token'
                },
                body: JSON.stringify({
                    peripheral_id: prompt.toUpperCase().replace(/\s+/g, '_'),
                    chip_yaml: `name: ${prompt}\ndescription: Generated from prompt`
                })
            });
            const data = await response.json();
            setRunId(data.run_id);
        } catch (err) {
            console.error('Generation failed:', err);
            setIsGenerating(false);
        }
    };

    useEffect(() => {
        if (!runId || !isGenerating) return;

        const poll = async () => {
            try {
                const response = await fetch(`http://localhost:8080/v1/runs/${runId}`, {
                    headers: { 'Authorization': 'Bearer local-dev-token' }
                });
                const data = await response.json();
                setStatus(data.status);

                if (['pass', 'fail', 'error'].includes(data.status)) {
                    setIsGenerating(false);
                }
            } catch (err) {
                console.error('Polling failed:', err);
            }
        };

        const interval = setInterval(poll, 2000);
        return () => clearInterval(interval);
    }, [runId, isGenerating]);

    return (
        <div style={{ padding: '2rem', flex: 1, maxWidth: '1000px' }}>
            <header style={{ marginBottom: '3rem' }}>
                <h1 style={{ fontSize: '3rem', color: 'var(--lw-black)' }}>FOUNDRY GEN</h1>
                <p style={{ color: 'var(--lw-gray)', marginTop: '0.5rem', fontSize: '1.1rem' }}>
                    Synthesize operational <strong>digital twins</strong> from documentation and prompts.
                </p>
            </header>

            <div className="bento-card" style={{ padding: '2.5rem' }}>
                <div style={{ marginBottom: '2rem' }}>
                    <label style={{ display: 'block', fontSize: '0.7rem', color: 'var(--lw-pink)', marginBottom: '12px', fontWeight: 900, letterSpacing: '0.1em' }}>
                        PERIPHERAL ARCHITECTURE PROMPT
                    </label>
                    <textarea
                        value={prompt}
                        onChange={(e) => setPrompt(e.target.value)}
                        disabled={isGenerating}
                        placeholder="e.g. A 3-axis accelerometer with I2C interface, 8-bit resolution, and interrupt on data ready."
                        className="mono"
                        style={{
                            width: '100%',
                            height: '160px',
                            background: 'var(--lw-bg-alt)',
                            border: 'var(--lw-border)',
                            borderRadius: '8px',
                            color: 'var(--lw-black)',
                            padding: '1.25rem',
                            fontSize: '1rem',
                            outline: 'none',
                            resize: 'none',
                            lineHeight: '1.6'
                        }}
                    />
                </div>

                <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                    <div style={{ fontSize: '0.8rem', color: 'var(--lw-gray)', fontWeight: 600 }}>
                        COST: <span style={{ color: 'var(--lw-pink)' }}>10 SIMULATION RUNS</span>
                    </div>
                    <button
                        onClick={handleGenerate}
                        disabled={isGenerating || !prompt}
                        style={{ minWidth: '220px', padding: '1rem 2rem' }}
                    >
                        {isGenerating ? 'ANALYZING...' : 'START SOLID PROOF'}
                    </button>
                </div>
            </div>

            {isGenerating && (
                <div className="bento-card" style={{ marginTop: '2rem', borderLeft: '6px solid var(--lw-pink)' }}>
                    <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '1rem' }}>
                        <h3 style={{ fontSize: '1rem', color: 'var(--lw-black)' }}>SIMULATION PIPELINE</h3>
                        <span className="text-pink mono" style={{ fontSize: '0.85rem', fontWeight: 800 }}>{status.toUpperCase()}</span>
                    </div>
                    <div style={{ height: '10px', width: '100%', background: 'var(--lw-bg-alt)', border: '1px solid #000', borderRadius: '5px', overflow: 'hidden' }}>
                        <div style={{
                            height: '100%',
                            width: status === 'queued' ? '20%' : (status === 'running' ? '60%' : '100%'),
                            background: 'var(--lw-pink)',
                            transition: 'width 0.5s cubic-bezier(0.4, 0, 0.2, 1)'
                        }}></div>
                    </div>
                </div>
            )}

            {status === 'pass' && !isGenerating && (
                <div className="bento-card" style={{ marginTop: '2rem', borderLeft: '6px solid var(--lw-green)' }}>
                    <h3 style={{ fontSize: '1.2rem', color: 'var(--lw-green)', marginBottom: '0.5rem' }}>PROVEN SUCCESSFUL</h3>
                    <p style={{ color: 'var(--lw-gray)', fontSize: '0.95rem' }}>The asset has been formally verified and committed to the registry.</p>
                    <div style={{ display: 'flex', gap: '12px', marginTop: '1.5rem' }}>
                        <button className="secondary">VIEW REPORT</button>
                        <button className="secondary">PUSH TO CI</button>
                    </div>
                </div>
            )}
        </div>
    );
};

export default Generator;
