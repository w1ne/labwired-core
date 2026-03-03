import { useState, useEffect } from 'react';

const UsageStats = () => {
    const [usage, setUsage] = useState<any>(null);

    useEffect(() => {
        fetch('http://localhost:8080/v1/usage', {
            headers: { 'Authorization': 'Bearer local-dev-token' }
        })
            .then(res => res.json())
            .then(data => setUsage(data))
            .catch(err => console.error('Failed to fetch usage:', err));
    }, []);

    if (!usage) return null;

    const percentage = (usage.runs_used_this_month / usage.quota) * 100;

    return (
        <div style={{ padding: '2rem', flex: 1 }}>
            <header style={{ marginBottom: '3rem' }}>
                <h1 style={{ fontSize: '3rem', color: 'var(--lw-black)' }}>USAGE & QUOTA</h1>
                <p style={{ color: 'var(--lw-gray)', marginTop: '0.5rem', fontSize: '1.1rem' }}>
                    Real-time resource utilization for the current billing cycle.
                </p>
            </header>

            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(400px, 1fr))', gap: '2rem' }}>
                <div className="bento-card">
                    <h4 style={{ fontSize: '0.7rem', color: 'var(--lw-gray)', marginBottom: '1.5rem', fontWeight: 900, letterSpacing: '0.1em' }}>SIMULATION QUOTA</h4>
                    <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '12px' }}>
                        <span style={{ fontSize: '0.85rem', color: 'var(--lw-gray)', fontWeight: 600 }}>RUNS CONSUMED</span>
                        <span className="mono" style={{ fontSize: '1.2rem', fontWeight: 900 }}>{usage.runs_used_this_month} / {usage.quota}</span>
                    </div>
                    <div style={{ height: '12px', width: '100%', background: 'var(--lw-bg-alt)', border: 'var(--lw-border)', borderRadius: '6px', overflow: 'hidden', marginBottom: '1.5rem' }}>
                        <div style={{
                            height: '100%',
                            width: `${percentage}%`,
                            background: 'var(--lw-pink)',
                            borderRadius: '6px'
                        }}></div>
                    </div>
                    <p style={{ fontSize: '0.85rem', color: 'var(--lw-gray)', fontWeight: 500 }}>
                        Usage reset in <span style={{ color: 'var(--lw-black)', fontWeight: 800 }}>14 DAYS</span>.
                    </p>
                </div>

                <div className="bento-card" style={{ display: 'flex', flexDirection: 'column', justifyContent: 'space-between' }}>
                    <h4 style={{ fontSize: '0.7rem', color: 'var(--lw-gray)', marginBottom: '1.5rem', fontWeight: 900, letterSpacing: '0.1em' }}>AI OPERATIONS</h4>
                    <div style={{ fontSize: '4rem', fontWeight: 900, fontFamily: 'Outfit', color: 'var(--lw-black)', lineHeight: 1 }}>142</div>
                    <p style={{ fontSize: '0.85rem', color: 'var(--lw-gray)', fontWeight: 500, marginTop: '1.5rem' }}>
                        Discrete LLM synthesis requests across all connected workspaces.
                    </p>
                </div>
            </div>
        </div>
    );
};

export default UsageStats;
