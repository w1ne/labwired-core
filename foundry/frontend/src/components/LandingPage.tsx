import { apiUrl, STRIPE_PAYMENT_LINK } from '../api';
import { Show, SignInButton, UserButton } from '@clerk/react';

const CURL_SNIPPET = `curl -X POST https://foundry.labwired.dev/v1/models/verify \\
  -H "Authorization: Bearer lw_sk_live_YOUR_KEY" \\
  -H "Content-Type: application/json" \\
  -d '{"chip_yaml": "device: ADXL345\\nregisters: ..."}'`;

interface Props {
    onEnterDashboard: () => void;
}

const LandingPage = ({ onEnterDashboard }: Props) => {
    const [copied, setCopied] = useState(false);
    const [assets, setAssets] = useState<any[]>([]);

    useEffect(() => {
        fetch(apiUrl('/v1/catalog'))
            .then(r => r.json())
            .then(setAssets)
            .catch(() => { });
    }, []);

    const handleCopy = () => {
        navigator.clipboard.writeText(CURL_SNIPPET);
        setCopied(true);
        setTimeout(() => setCopied(false), 2000);
    };

    return (
        <div style={{ minHeight: '100vh', background: 'var(--lw-bg)', fontFamily: 'Inter, sans-serif' }}>
            {/* NAV */}
            <nav style={{
                display: 'flex', justifyContent: 'space-between', alignItems: 'center',
                padding: '1.2rem 2.5rem', borderBottom: 'var(--lw-border)',
                background: 'var(--lw-bg)', position: 'sticky', top: 0, zIndex: 100,
            }}>
                <div style={{ fontFamily: 'Outfit', fontWeight: 900, fontSize: '1.3rem', letterSpacing: '-0.03em' }}>
                    LABWIRED <span style={{ color: 'var(--lw-pink)' }}>FOUNDRY</span>
                </div>
                <div style={{ display: 'flex', gap: '1rem', alignItems: 'center' }}>
                    <a href="https://api.labwired.com/v1/docs" target="_blank" rel="noopener noreferrer" style={{ color: 'var(--lw-gray)', textDecoration: 'none', fontWeight: 600, marginRight: '0.5rem' }}>API Docs</a>
                    <button className="secondary" onClick={() => window.location.hash = '/catalog'}>Catalog</button>
                    <Show when="signed-out">
                        <SignInButton mode="modal" fallbackRedirectUrl="#/dashboard">
                            <button>Dashboard →</button>
                        </SignInButton>
                    </Show>
                    <Show when="signed-in">
                        <button onClick={onEnterDashboard}>Dashboard →</button>
                        <UserButton />
                    </Show>
                </div>
            </nav>

            {/* HERO */}
            <section style={{
                maxWidth: '900px', margin: '0 auto', padding: '7rem 2rem 5rem',
                textAlign: 'center',
            }}>
                <div style={{
                    display: 'inline-block', background: 'var(--lw-black)', color: 'var(--lw-bg)',
                    fontSize: '0.7rem', fontWeight: 800, letterSpacing: '0.15em',
                    padding: '6px 14px', borderRadius: '4px', marginBottom: '2rem',
                }}>
                    AGENT-NATIVE SIMULATION API
                </div>

                <h1 style={{
                    fontFamily: 'Outfit', fontSize: 'clamp(2.5rem, 6vw, 4.5rem)', fontWeight: 900,
                    lineHeight: 1.05, letterSpacing: '-0.04em', marginBottom: '1.5rem',
                }}>
                    Formally proven hardware<br />
                    simulation. <span style={{ color: 'var(--lw-pink)' }}>One API call.</span>
                </h1>

                <p style={{ fontSize: '1.2rem', color: 'var(--lw-gray)', maxWidth: '600px', margin: '0 auto 3rem', lineHeight: 1.7 }}>
                    Transform peripheral datasheets into verified digital twins.
                    Generate once. Run forever on your local CI — free.
                </p>

                <div style={{ display: 'flex', gap: '1rem', justifyContent: 'center', flexWrap: 'wrap' }}>
                    <button onClick={onEnterDashboard} style={{ fontSize: '1rem', padding: '1rem 2.5rem' }}>
                        Get your free API key →
                    </button>
                    <button className="secondary" onClick={() => window.location.hash = '/catalog'} style={{ fontSize: '1rem', padding: '1rem 2.5rem' }}>
                        Browse catalog
                    </button>
                </div>
            </section>

            {/* CURL DEMO */}
            <section style={{ maxWidth: '860px', margin: '0 auto 6rem', padding: '0 2rem' }}>
                <div style={{
                    background: '#0d1117', border: '1px solid #30363d', borderRadius: '12px',
                    overflow: 'hidden', boxShadow: 'var(--lw-shadow-lg)',
                }}>
                    <div style={{
                        display: 'flex', justifyContent: 'space-between', alignItems: 'center',
                        padding: '0.75rem 1.25rem', borderBottom: '1px solid #30363d',
                        background: '#161b22',
                    }}>
                        <span style={{ color: '#8b949e', fontSize: '0.8rem', fontFamily: 'JetBrains Mono, monospace' }}>
                            30-second quickstart
                        </span>
                        <button
                            onClick={handleCopy}
                            className="secondary"
                            style={{
                                background: 'transparent', border: '1px solid #30363d',
                                color: copied ? '#39ff14' : '#8b949e', fontSize: '0.75rem',
                                padding: '4px 12px', boxShadow: 'none',
                            }}
                        >
                            {copied ? '✓ COPIED' : 'COPY'}
                        </button>
                    </div>
                    <pre style={{
                        margin: 0, padding: '1.5rem', overflowX: 'auto',
                        fontFamily: 'JetBrains Mono, monospace', fontSize: '0.85rem',
                        lineHeight: 1.7, color: '#e6edf3',
                    }}>
                        {CURL_SNIPPET}
                    </pre>
                </div>
            </section>

            {/* CATALOG PREVIEW */}
            <section style={{ maxWidth: '1100px', margin: '0 auto 6rem', padding: '0 2rem' }}>
                <h2 style={{ fontFamily: 'Outfit', fontSize: '2rem', fontWeight: 900, marginBottom: '0.5rem' }}>
                    Pre-verified catalog
                </h2>
                <p style={{ color: 'var(--lw-gray)', marginBottom: '2.5rem' }}>
                    Download and use immediately — no simulation credits needed.
                </p>
                <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(300px, 1fr))', gap: '1.5rem' }}>
                    {assets.slice(0, 6).map((asset: any) => (
                        <div
                            key={asset.id}
                            className="bento-card"
                            style={{ cursor: 'pointer' }}
                            onClick={() => { window.location.hash = `/assets/${asset.id}`; }}
                        >
                            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '0.75rem' }}>
                                <h3 style={{ fontSize: '1.25rem' }}>{asset.id}</h3>
                                <span className="status-pill" style={{ color: 'var(--lw-green)', borderColor: 'var(--lw-green)', fontSize: '0.65rem' }}>
                                    ✓ PROVEN
                                </span>
                            </div>
                            <p style={{ color: 'var(--lw-gray)', fontSize: '0.9rem', marginBottom: '1rem', lineHeight: 1.5 }}>
                                {asset.description || 'Pre-verified peripheral model.'}
                            </p>
                            <div style={{ display: 'flex', gap: '1.5rem', fontSize: '0.8rem', color: 'var(--lw-gray)' }}>
                                <span><strong style={{ color: 'var(--lw-black)' }}>{asset.registers ?? '—'}</strong> registers</span>
                                <span><strong style={{ color: 'var(--lw-pink)' }}>{asset.pass_rate ?? 100}%</strong> pass rate</span>
                            </div>
                        </div>
                    ))}
                </div>
                {assets.length === 0 && (
                    <p style={{ color: 'var(--lw-gray)', fontStyle: 'italic' }}>Start the backend to see the live catalog.</p>
                )}
            </section>

            {/* PRICING TEASER */}
            <section style={{
                background: 'var(--lw-black)', color: 'var(--lw-bg)',
                padding: '5rem 2rem', textAlign: 'center',
            }}>
                <h2 style={{ fontFamily: 'Outfit', fontSize: '2.5rem', marginBottom: '1rem', color: 'var(--lw-bg)' }}>
                    Simple, transparent pricing
                </h2>
                <p style={{ color: '#8b949e', maxWidth: '500px', margin: '0 auto 0.5rem' }}>
                    Pay per simulation run. No seat fees.
                </p>
                <p style={{ color: 'var(--lw-pink)', fontWeight: 700, marginBottom: '3rem' }}>
                    1,000 runs / €49 · burst at €0.05/run
                </p>
                <div style={{ display: 'flex', gap: '1rem', justifyContent: 'center', flexWrap: 'wrap' }}>
                    <button onClick={onEnterDashboard} style={{ background: 'var(--lw-pink)', border: 'none', color: '#fff', padding: '1rem 2.5rem', fontSize: '1rem' }}>
                        Start free
                    </button>
                    <a href={STRIPE_PAYMENT_LINK} target="_blank" rel="noopener noreferrer">
                        <button className="secondary" style={{ background: 'transparent', color: 'var(--lw-bg)', border: '2px solid var(--lw-bg)', padding: '1rem 2.5rem', fontSize: '1rem' }}>
                            Buy 1,000 runs →
                        </button>
                    </a>
                </div>
            </section>

            {/* FOOTER */}
            <footer style={{ borderTop: 'var(--lw-border)', padding: '1.5rem 2.5rem', display: 'flex', justifyContent: 'space-between', fontSize: '0.8rem', color: 'var(--lw-gray)' }}>
                <span>© 2026 LabWired</span>
                <a href="https://api.labwired.com/v1/openapi.yaml" style={{ color: 'var(--lw-gray)', textDecoration: 'none' }}>OpenAPI spec</a>
            </footer>
        </div>
    );
};

// Need to add useState, useEffect imports
import { useState, useEffect } from 'react';
export default LandingPage;
