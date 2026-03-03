import { useEffect, useState } from 'react';

interface Asset {
    id: string;
    name: string;
    description: string;
    pass_rate: number;
    registers: number;
    ir_url: string;
}

const AssetCatalog = () => {
    const [assets, setAssets] = useState<Asset[]>([]);
    const [loading, setLoading] = useState(true);

    useEffect(() => {
        fetch('http://localhost:8080/v1/catalog', {
            headers: { 'Authorization': 'Bearer local-dev-token' }
        })
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
        <div style={{ padding: '2rem', flex: 1 }}>
            <header style={{ marginBottom: '3rem' }}>
                <h1 style={{ fontSize: '3rem', color: 'var(--lw-black)' }}>VERIFIED ASSETS</h1>
                <p style={{ color: 'var(--lw-gray)', marginTop: '0.5rem', maxWidth: '600px', fontSize: '1.1rem' }}>
                    Pre-verified <strong>Strict IR</strong> models for bit-accurate MCU simulation.
                </p>
            </header>

            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(340px, 1fr))', gap: '2rem' }}>
                {assets.map(asset => (
                    <div key={asset.id} className="bento-card">
                        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start', marginBottom: '1.5rem' }}>
                            <h3 style={{ fontSize: '1.5rem', color: 'var(--lw-black)' }}>{asset.id}</h3>
                            <div className="status-pill" style={{ color: 'var(--lw-green)', borderColor: 'var(--lw-black)', background: 'var(--lw-bg)' }}>
                                SOLID PROVEN
                            </div>
                        </div>

                        <p style={{ fontSize: '0.95rem', color: 'var(--lw-gray)', marginBottom: '2rem', minHeight: '3.5rem', lineHeight: '1.6' }}>
                            {asset.description}
                        </p>

                        <div style={{ display: 'flex', gap: '2.5rem', marginBottom: '2rem', borderTop: '1px solid #eee', paddingTop: '1.5rem' }}>
                            <div>
                                <div style={{ fontSize: '0.7rem', color: 'var(--lw-gray)', textTransform: 'uppercase', fontWeight: 800 }}>Registers</div>
                                <div style={{ fontSize: '1.5rem', fontWeight: 900, fontFamily: 'Outfit' }}>{asset.registers}</div>
                            </div>
                            <div>
                                <div style={{ fontSize: '0.7rem', color: 'var(--lw-gray)', textTransform: 'uppercase', fontWeight: 800 }}>Pass Rate</div>
                                <div style={{ fontSize: '1.5rem', fontWeight: 900, fontFamily: 'Outfit', color: 'var(--lw-pink)' }}>{asset.pass_rate}%</div>
                            </div>
                        </div>

                        <div style={{ display: 'flex', gap: '12px' }}>
                            <button style={{ flex: 1 }}>SIMULATE</button>
                            <button className="secondary" style={{ flex: 1 }}>SPEC</button>
                        </div>
                    </div>
                ))}
            </div>
        </div>
    );
};

export default AssetCatalog;
