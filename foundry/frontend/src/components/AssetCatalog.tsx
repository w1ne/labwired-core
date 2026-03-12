import { useEffect, useMemo, useState } from 'react';
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
    source_url?: string;
    official_url?: string;
    validation_url?: string;
}

interface Props {
    onSelectAsset?: (id: string) => void;
}

type SortKey = 'name' | 'pass_rate' | 'registers' | 'source_type';

const AssetCatalog = ({ onSelectAsset }: Props) => {
    const [assets, setAssets] = useState<Asset[]>([]);
    const [loading, setLoading] = useState(true);
    const [query, setQuery] = useState('');
    const [sourceFilter, setSourceFilter] = useState('all');
    const [verifiedOnly, setVerifiedOnly] = useState(false);
    const [sortKey, setSortKey] = useState<SortKey>('name');
    const [sortAsc, setSortAsc] = useState(true);

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

    const sourceTypes = useMemo(() => {
        const unique = new Set(assets.map(a => a.source_type || 'unknown'));
        return ['all', ...Array.from(unique).sort()];
    }, [assets]);

    const filteredAssets = useMemo(() => {
        const q = query.trim().toLowerCase();
        const rows = assets.filter(asset => {
            if (verifiedOnly && !asset.verified) return false;
            if (sourceFilter !== 'all' && (asset.source_type || 'unknown') !== sourceFilter) return false;
            if (!q) return true;
            return (
                asset.id.toLowerCase().includes(q) ||
                asset.name.toLowerCase().includes(q) ||
                asset.description.toLowerCase().includes(q) ||
                (asset.architecture || '').toLowerCase().includes(q)
            );
        });

        rows.sort((a, b) => {
            let cmp = 0;
            switch (sortKey) {
                case 'pass_rate':
                    cmp = (a.pass_rate || 0) - (b.pass_rate || 0);
                    break;
                case 'registers':
                    cmp = (a.registers || 0) - (b.registers || 0);
                    break;
                case 'source_type':
                    cmp = (a.source_type || '').localeCompare(b.source_type || '');
                    break;
                case 'name':
                default:
                    cmp = (a.name || '').localeCompare(b.name || '');
                    break;
            }
            return sortAsc ? cmp : -cmp;
        });

        return rows;
    }, [assets, query, sourceFilter, verifiedOnly, sortKey, sortAsc]);

    const setSort = (key: SortKey) => {
        if (sortKey === key) {
            setSortAsc(prev => !prev);
        } else {
            setSortKey(key);
            setSortAsc(true);
        }
    };

    if (loading) {
        return <div style={{ padding: '2rem', color: 'var(--lw-pink)', fontWeight: 800, fontFamily: 'Outfit' }}>SCANNING LABWIRED CATALOG...</div>;
    }

    return (
        <div style={{ padding: '2rem', flex: 1, backgroundColor: '#f9f9f9' }}>
            <header style={{ marginBottom: '1.5rem' }}>
                <h1 style={{ fontSize: '2.5rem', color: 'var(--lw-black)', marginBottom: '0.5rem' }}>VERIFIED ASSETS</h1>
                <p style={{ color: 'var(--lw-gray)', marginTop: 0, maxWidth: '780px', fontSize: '1rem' }}>
                    Unified board catalog with searchable metadata and direct links to source, vendor pages, and latest validation evidence.
                </p>
            </header>

            <div className="bento-card" style={{ marginBottom: '1.5rem', display: 'grid', gap: '0.8rem', gridTemplateColumns: 'repeat(auto-fit, minmax(190px, 1fr))' }}>
                <input
                    type="text"
                    value={query}
                    onChange={e => setQuery(e.target.value)}
                    placeholder="Search by id, name, architecture..."
                    style={{ padding: '0.7rem 0.8rem', borderRadius: '8px', border: '1px solid #ddd' }}
                />
                <select value={sourceFilter} onChange={e => setSourceFilter(e.target.value)} style={{ padding: '0.7rem 0.8rem', borderRadius: '8px', border: '1px solid #ddd' }}>
                    {sourceTypes.map(source => (
                        <option key={source} value={source}>{source === 'all' ? 'All sources' : source}</option>
                    ))}
                </select>
                <label style={{ display: 'flex', alignItems: 'center', gap: '0.5rem', color: 'var(--lw-gray)', fontWeight: 600 }}>
                    <input type="checkbox" checked={verifiedOnly} onChange={e => setVerifiedOnly(e.target.checked)} />
                    Verified only
                </label>
                <div style={{ color: 'var(--lw-gray)', fontWeight: 600, alignSelf: 'center' }}>
                    Showing {filteredAssets.length} of {assets.length}
                </div>
            </div>

            {assets.length === 0 && <p style={{ color: 'var(--lw-gray)', fontStyle: 'italic' }}>No assets found. The backend may not be running.</p>}

            <div className="bento-card" style={{ padding: 0, overflowX: 'auto' }}>
                <table style={{ width: '100%', borderCollapse: 'collapse', minWidth: '980px' }}>
                    <thead>
                        <tr style={{ backgroundColor: '#f3f4fb', borderBottom: '1px solid #ddd' }}>
                            <th style={{ textAlign: 'left', padding: '0.8rem' }}>ID</th>
                            <th style={{ textAlign: 'left', padding: '0.8rem', cursor: 'pointer' }} onClick={() => setSort('name')}>Name</th>
                            <th style={{ textAlign: 'left', padding: '0.8rem' }}>Arch</th>
                            <th style={{ textAlign: 'right', padding: '0.8rem', cursor: 'pointer' }} onClick={() => setSort('pass_rate')}>Pass %</th>
                            <th style={{ textAlign: 'right', padding: '0.8rem', cursor: 'pointer' }} onClick={() => setSort('registers')}>Registers</th>
                            <th style={{ textAlign: 'left', padding: '0.8rem', cursor: 'pointer' }} onClick={() => setSort('source_type')}>Source</th>
                            <th style={{ textAlign: 'left', padding: '0.8rem' }}>Links</th>
                            <th style={{ textAlign: 'left', padding: '0.8rem' }}>Action</th>
                        </tr>
                    </thead>
                    <tbody>
                        {filteredAssets.map(asset => (
                            <tr key={asset.id} style={{ borderBottom: '1px solid #eee' }}>
                                <td style={{ padding: '0.75rem', fontFamily: 'JetBrains Mono, monospace', fontSize: '0.82rem' }}>{asset.id}</td>
                                <td style={{ padding: '0.75rem' }}>
                                    <div style={{ fontWeight: 700 }}>{asset.name}</div>
                                    <div style={{ color: 'var(--lw-gray)', fontSize: '0.82rem' }}>{asset.description}</div>
                                </td>
                                <td style={{ padding: '0.75rem', color: 'var(--lw-gray)' }}>{asset.architecture || '—'}</td>
                                <td style={{ padding: '0.75rem', textAlign: 'right', color: asset.verified ? 'var(--lw-pink)' : 'var(--lw-gray)', fontWeight: 700 }}>
                                    {asset.verified ? `${asset.pass_rate}%` : 'N/A'}
                                </td>
                                <td style={{ padding: '0.75rem', textAlign: 'right', fontWeight: 700 }}>{asset.registers || 0}</td>
                                <td style={{ padding: '0.75rem', color: 'var(--lw-gray)' }}>{asset.source_type || 'unknown'}</td>
                                <td style={{ padding: '0.75rem' }}>
                                    <div style={{ display: 'flex', gap: '0.75rem', flexWrap: 'wrap', fontSize: '0.82rem' }}>
                                        {asset.source_url && <a href={asset.source_url} target="_blank" rel="noreferrer">Source</a>}
                                        {asset.official_url && <a href={asset.official_url} target="_blank" rel="noreferrer">Official</a>}
                                        {asset.validation_url && <a href={asset.validation_url} target="_blank" rel="noreferrer">Validation</a>}
                                    </div>
                                </td>
                                <td style={{ padding: '0.75rem' }}>
                                    <button className="secondary" onClick={() => onSelectAsset?.(asset.id)}>Details</button>
                                </td>
                            </tr>
                        ))}
                    </tbody>
                </table>
            </div>
        </div>
    );
};

export default AssetCatalog;
