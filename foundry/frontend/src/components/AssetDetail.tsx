import { useState, useEffect } from 'react';
import { apiUrl } from '../api';

interface Props {
    id: string;
    onBack: () => void;
}

const AssetDetail = ({ id, onBack }: Props) => {
    const [asset, setAsset] = useState<any>(null);
    const [loading, setLoading] = useState(true);

    useEffect(() => {
        fetch(apiUrl(`/v1/catalog/${id}`))
            .then(r => r.json())
            .then(data => { setAsset(data); setLoading(false); })
            .catch(() => setLoading(false));
    }, [id]);

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

    return (
        <div style={{ minHeight: '100vh', background: 'var(--lw-bg)', padding: '2rem 2.5rem', maxWidth: '960px', margin: '0 auto' }}>
            <button className="secondary" onClick={onBack} style={{ marginBottom: '2rem' }}>← Back to catalog</button>

            {/* Proof badge */}
            <div style={{ display: 'flex', alignItems: 'center', gap: '1.5rem', marginBottom: '2rem', flexWrap: 'wrap' }}>
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

            <p style={{ color: 'var(--lw-gray)', fontSize: '1.1rem', marginBottom: '3rem', maxWidth: '600px', lineHeight: 1.7 }}>
                {asset.description || 'Pre-verified peripheral model.'}
            </p>

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
            <p style={{ color: 'var(--lw-gray)', marginTop: '-0.25rem', marginBottom: '1rem', fontSize: '0.9rem' }}>
                Source: {asset.source_type || 'unknown'} {asset.source_ref ? `(${asset.source_ref})` : ''}
            </p>
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
