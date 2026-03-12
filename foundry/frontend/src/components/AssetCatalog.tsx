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

type SortKey = 'name' | 'pass_rate' | 'registers' | 'architecture' | 'source_type';
type StatusFilter = 'all' | 'verified' | 'modeled';

type CatalogRow = Asset & {
    category: string;
    arch: string;
};

const categoryFromID = (id: string): string => {
    const part = (id || '').split('/')[0] || 'unknown';
    if (!part) return 'unknown';
    return part;
};

const normalizedArchitecture = (arch: string): string => {
    const trimmed = (arch || '').trim();
    return trimmed === '' ? 'unknown' : trimmed;
};

const scoreLabel = (asset: Asset): string => {
    if (!asset.verified) return 'MODELED';
    if (asset.pass_rate >= 100) return 'VERIFIED';
    if (asset.pass_rate >= 70) return 'PARTIAL';
    return 'LOW';
};

const compareText = (a: string, b: string): number => a.localeCompare(b, undefined, { sensitivity: 'base' });

const AssetCatalog = ({ onSelectAsset }: Props) => {
    const [assets, setAssets] = useState<Asset[]>([]);
    const [loading, setLoading] = useState(true);

    const [query, setQuery] = useState('');
    const [categoryFilter, setCategoryFilter] = useState('all');
    const [sourceFilter, setSourceFilter] = useState('all');
    const [archFilter, setArchFilter] = useState('all');
    const [statusFilter, setStatusFilter] = useState<StatusFilter>('all');

    const [sortKey, setSortKey] = useState<SortKey>('name');
    const [sortAsc, setSortAsc] = useState(true);
    const [page, setPage] = useState(1);
    const [pageSize, setPageSize] = useState(30);

    useEffect(() => {
        fetch(apiUrl('/v1/catalog'), { headers: authHeaders() })
            .then(res => res.json())
            .then(data => {
                setAssets(Array.isArray(data) ? data : []);
                setLoading(false);
            })
            .catch(err => {
                console.error('Failed to fetch catalog:', err);
                setLoading(false);
            });
    }, []);

    const rows: CatalogRow[] = useMemo(() => {
        return assets.map(a => ({
            ...a,
            category: categoryFromID(a.id),
            arch: normalizedArchitecture(a.architecture),
        }));
    }, [assets]);

    const categories = useMemo(() => ['all', ...Array.from(new Set(rows.map(r => r.category))).sort(compareText)], [rows]);
    const sourceTypes = useMemo(() => ['all', ...Array.from(new Set(rows.map(r => r.source_type || 'unknown'))).sort(compareText)], [rows]);
    const archValues = useMemo(() => ['all', ...Array.from(new Set(rows.map(r => r.arch))).sort(compareText)], [rows]);

    const filteredRows = useMemo(() => {
        const q = query.trim().toLowerCase();
        const filtered = rows.filter(row => {
            if (categoryFilter !== 'all' && row.category !== categoryFilter) return false;
            if (sourceFilter !== 'all' && (row.source_type || 'unknown') !== sourceFilter) return false;
            if (archFilter !== 'all' && row.arch !== archFilter) return false;
            if (statusFilter === 'verified' && !row.verified) return false;
            if (statusFilter === 'modeled' && row.verified) return false;

            if (!q) return true;
            return (
                row.id.toLowerCase().includes(q) ||
                row.name.toLowerCase().includes(q) ||
                row.description.toLowerCase().includes(q) ||
                row.arch.toLowerCase().includes(q) ||
                row.category.toLowerCase().includes(q)
            );
        });

        filtered.sort((a, b) => {
            let cmp = 0;
            switch (sortKey) {
                case 'pass_rate':
                    cmp = (a.pass_rate || 0) - (b.pass_rate || 0);
                    break;
                case 'registers':
                    cmp = (a.registers || 0) - (b.registers || 0);
                    break;
                case 'architecture':
                    cmp = compareText(a.arch, b.arch);
                    break;
                case 'source_type':
                    cmp = compareText(a.source_type || '', b.source_type || '');
                    break;
                case 'name':
                default:
                    cmp = compareText(a.name || '', b.name || '');
                    break;
            }
            return sortAsc ? cmp : -cmp;
        });

        return filtered;
    }, [rows, query, categoryFilter, sourceFilter, archFilter, statusFilter, sortKey, sortAsc]);

    const totalPages = Math.max(1, Math.ceil(filteredRows.length / pageSize));
    const currentPage = Math.min(page, totalPages);
    const pagedRows = useMemo(() => {
        const start = (currentPage - 1) * pageSize;
        return filteredRows.slice(start, start + pageSize);
    }, [filteredRows, currentPage, pageSize]);

    useEffect(() => {
        setPage(1);
    }, [query, categoryFilter, sourceFilter, archFilter, statusFilter, sortKey, sortAsc, pageSize]);

    const setSort = (key: SortKey) => {
        if (sortKey === key) {
            setSortAsc(prev => !prev);
        } else {
            setSortKey(key);
            setSortAsc(true);
        }
    };

    const verifiedCount = rows.filter(r => r.verified).length;

    if (loading) {
        return <div style={{ padding: '2rem', color: 'var(--lw-pink)', fontWeight: 800, fontFamily: 'Outfit' }}>SCANNING LABWIRED CATALOG...</div>;
    }

    return (
        <div className="catalog-page">
            <header className="catalog-header">
                <h1>ASSET CATALOG</h1>
                <p>Search, filter, and sort across boards/chips/peripherals with source, official docs, and validation evidence links.</p>
            </header>

            <section className="catalog-metrics">
                <div className="catalog-metric"><span>Total</span><strong>{rows.length}</strong></div>
                <div className="catalog-metric"><span>Verified</span><strong>{verifiedCount}</strong></div>
                <div className="catalog-metric"><span>Sources</span><strong>{sourceTypes.length - 1}</strong></div>
                <div className="catalog-metric"><span>Architectures</span><strong>{archValues.length - 1}</strong></div>
            </section>

            <section className="catalog-controls bento-card">
                <input
                    type="text"
                    value={query}
                    onChange={e => setQuery(e.target.value)}
                    placeholder="Search id, name, category, architecture..."
                />
                <select value={categoryFilter} onChange={e => setCategoryFilter(e.target.value)}>
                    {categories.map(value => <option key={value} value={value}>{value === 'all' ? 'All categories' : value}</option>)}
                </select>
                <select value={archFilter} onChange={e => setArchFilter(e.target.value)}>
                    {archValues.map(value => <option key={value} value={value}>{value === 'all' ? 'All architectures' : value}</option>)}
                </select>
                <select value={sourceFilter} onChange={e => setSourceFilter(e.target.value)}>
                    {sourceTypes.map(value => <option key={value} value={value}>{value === 'all' ? 'All sources' : value}</option>)}
                </select>
                <select value={statusFilter} onChange={e => setStatusFilter(e.target.value as StatusFilter)}>
                    <option value="all">All quality states</option>
                    <option value="verified">Verified only</option>
                    <option value="modeled">Modeled only</option>
                </select>
                <select value={pageSize} onChange={e => setPageSize(Number(e.target.value))}>
                    <option value={20}>20 rows/page</option>
                    <option value={30}>30 rows/page</option>
                    <option value={50}>50 rows/page</option>
                    <option value={100}>100 rows/page</option>
                </select>
            </section>

            <section className="catalog-table-wrap bento-card">
                {rows.length === 0 ? (
                    <p style={{ color: 'var(--lw-gray)', fontStyle: 'italic' }}>No assets found. The backend may not be running.</p>
                ) : (
                    <table className="catalog-table">
                        <thead>
                            <tr>
                                <th>ID</th>
                                <th onClick={() => setSort('name')}>Name</th>
                                <th>Category</th>
                                <th onClick={() => setSort('architecture')}>Architecture</th>
                                <th onClick={() => setSort('pass_rate')}>Quality</th>
                                <th onClick={() => setSort('registers')}>Registers</th>
                                <th onClick={() => setSort('source_type')}>Source</th>
                                <th>Links</th>
                                <th>Action</th>
                            </tr>
                        </thead>
                        <tbody>
                            {pagedRows.map(asset => (
                                <tr key={asset.id}>
                                    <td className="mono">{asset.id}</td>
                                    <td>
                                        <div className="catalog-name">{asset.name}</div>
                                        <div className="catalog-desc">{asset.description}</div>
                                    </td>
                                    <td>{asset.category}</td>
                                    <td>{asset.arch}</td>
                                    <td>
                                        <span className={`catalog-score ${asset.verified ? 'ok' : 'warn'}`}>{scoreLabel(asset)} {asset.verified ? `${asset.pass_rate}%` : ''}</span>
                                    </td>
                                    <td>{asset.registers || 0}</td>
                                    <td>{asset.source_type || 'unknown'}</td>
                                    <td>
                                        <div className="catalog-links">
                                            {asset.source_url && <a href={asset.source_url} target="_blank" rel="noreferrer">Source</a>}
                                            {asset.official_url && <a href={asset.official_url} target="_blank" rel="noreferrer">Official</a>}
                                            {asset.validation_url && <a href={asset.validation_url} target="_blank" rel="noreferrer">Validation</a>}
                                        </div>
                                    </td>
                                    <td><button className="secondary" onClick={() => onSelectAsset?.(asset.id)}>Details</button></td>
                                </tr>
                            ))}
                        </tbody>
                    </table>
                )}
            </section>

            <section className="catalog-pagination">
                <button className="secondary" disabled={currentPage <= 1} onClick={() => setPage(currentPage - 1)}>Prev</button>
                <span>Page {currentPage} / {totalPages} • Showing {pagedRows.length} of {filteredRows.length}</span>
                <button className="secondary" disabled={currentPage >= totalPages} onClick={() => setPage(currentPage + 1)}>Next</button>
            </section>
        </div>
    );
};

export default AssetCatalog;
