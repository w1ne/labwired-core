import { useEffect, useState } from 'react';
import { apiUrl, authHeaders } from '../api';

interface Asset {
    id: string;
    name: string;
    description: string;
    architecture: string;
    pass_rate: number;
    registers: number;
    ir_url: string;
    verified: boolean;
    source_type: string;
    source_ref: string;
}

interface Props {
    onSelectAsset?: (id: string) => void;
}

const AssetCatalog = ({ onSelectAsset }: Props) => {
    const [assets, setAssets] = useState<Asset[]>([]);
    const [loading, setLoading] = useState(true);

    useEffect(() => {
        fetch(apiUrl('/v1/catalog'), { headers: authHeaders() })
            .then(res => res.json())
            .then(data => {
                setAssets(data);
                setLoading(false);
            })
            .catch(err => {
                console.error('Failed to fetch catalog:', err);
                setLoading(false);
            });
    }, []);

    if (loading) {
        return <div style={{ padding: '2rem', color: 'var(--lw-pink)', fontWeight: 800, fontFamily: 'Outfit' }}>SCANNING LABWIRED CATALOG...</div>;
    }

    return (
        <div style={{ padding: '2rem', flex: 1, backgroundColor: '#f9f9f9' }}>
            <header style={{ marginBottom: '3rem' }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: '1rem' }}>
                    <h1 style={{ fontSize: '3rem', color: 'var(--lw-black)' }}>VERIFIED ASSETS</h1>
                </div>
                <p style={{ color: 'var(--lw-gray)', marginTop: '0.5rem', maxWidth: '600px', fontSize: '1.1rem' }}>
                    Pre-verified <strong>Strict IR</strong> models for bit-accurate MCU simulation, automatically synced with upstream CI dashboards.
                </p>
            </header>

            {assets.length === 0 && (
                <p style={{ color: 'var(--lw-gray)', fontStyle: 'italic' }}>No assets found. The backend may not be running.</p>
            )}

            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(340px, 1fr))', gap: '2rem' }}>
                {assets.map(asset => (
                    <div key={asset.id} className="bento-card">
                        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start', marginBottom: '1.5rem' }}>
                            <div style={{ display: 'flex', flexDirection: 'column', gap: '0.5rem' }}>
                                <h3 style={{ fontSize: '1.5rem', color: 'var(--lw-black)', margin: 0 }}>{asset.name}</h3>
                                {asset.architecture && (
                                    <div style={{ fontSize: '0.8rem', color: 'var(--lw-blue)', fontWeight: 600 }}>{asset.architecture}</div>
                                )}
                            </div>

                            <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'flex-end', gap: '0.8rem' }}>
                                <div style={{
                                    backgroundColor: '#E6E9FC', color: 'var(--lw-blue)', padding: '0.2rem 1rem',
                                    borderRadius: '20px', fontSize: '0.75rem', fontWeight: 800, letterSpacing: '0.5px'
                                }}>
                                    PASSED
                                </div>
                                <div style={{ display: 'flex', gap: '0.7rem', color: 'var(--lw-blue)', opacity: 0.8, fontSize: '1.1rem' }}>
                                    <i className="fa-solid fa-play" style={{ cursor: 'pointer' }} title="Run in Sandbox" onClick={() => onSelectAsset?.(asset.id)}></i>
                                    <i className="fa-solid fa-file-lines" style={{ cursor: 'pointer' }} title="View Logs"></i>
                                    <i className="fa-brands fa-github" style={{ cursor: 'pointer' }} title="View Source"></i>
                                </div>
                            </div>
                        </div>

                        <p style={{ fontSize: '0.95rem', color: 'var(--lw-gray)', marginBottom: '2rem', minHeight: '3.5rem', lineHeight: '1.6' }}>
                            {asset.description.length > 120 ? asset.description.substring(0, 120) + '...' : asset.description}
                        </p>

                        <div style={{ display: 'flex', gap: '2.5rem', marginBottom: '2rem', borderTop: '1px solid #eee', paddingTop: '1.5rem' }}>
                            <div>
                                <div style={{ fontSize: '0.7rem', color: 'var(--lw-gray)', textTransform: 'uppercase', fontWeight: 800 }}>Registers</div>
                                <div style={{ fontSize: '1.5rem', fontWeight: 900, fontFamily: 'Outfit' }}>{asset.registers}</div>
                            </div>
                            <div>
                                <div style={{ fontSize: '0.7rem', color: 'var(--lw-gray)', textTransform: 'uppercase', fontWeight: 800 }}>Pass Rate</div>
                                <div style={{ fontSize: '1.5rem', fontWeight: 900, fontFamily: 'Outfit', color: asset.verified ? 'var(--lw-pink)' : 'var(--lw-gray)' }}>
                                    {asset.verified ? `${asset.pass_rate}%` : 'N/A'}
                                </div>
                            </div>
                        </div>

                        <p style={{ margin: '0 0 1rem', color: 'var(--lw-gray)', fontSize: '0.75rem' }}>
                            Source: {asset.source_type || 'unknown'} {asset.source_ref ? `(${asset.source_ref})` : ''}
                        </p>

                        <div style={{ display: 'flex', gap: '12px' }}>
                            <button style={{ flex: 1 }} onClick={() => onSelectAsset?.(asset.id)}>DETAILS</button>
                            {asset.ir_url && (
                                <a href={asset.ir_url} download style={{ flex: 1 }}>
                                    <button className="secondary" style={{ width: '100%' }}>DOWNLOAD IR</button>
                                </a>
                            )}
                        </div>
                    </div>
                ))}
            </div>
        </div>
    );
};

export default AssetCatalog;
