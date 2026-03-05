

const Sidebar = ({ activeTab, setActiveTab }: { activeTab: string, setActiveTab: (tab: string) => void }) => {
    const tabs = [
        { id: 'catalog', label: 'Asset Catalog', icon: '📦' },
        { id: 'usage', label: 'Usage & Quota', icon: '📊' },
        { id: 'health', label: 'Platform Health', icon: '🩺' },
        { id: 'docs', label: 'API Reference', icon: '📕' },
    ];

    return (
        <div className="bento-card" style={{ width: '280px', height: 'calc(100vh - 40px)', margin: '20px', display: 'flex', flexDirection: 'column', padding: '0' }}>
            <div style={{ padding: '2rem', borderBottom: 'var(--lw-border)' }}>
                <h2 style={{ fontSize: '1.5rem', fontWeight: 900, color: 'var(--lw-black)' }}>LABWIRED</h2>
                <div style={{ fontSize: '0.7rem', color: 'var(--lw-pink)', letterSpacing: '0.2em', marginTop: '4px', fontWeight: 800 }}>FOUNDRY / v0.1</div>
            </div>

            <nav style={{ flex: 1, padding: '1rem' }}>
                {tabs.map((tab) => (
                    <div
                        key={tab.id}
                        onClick={() => {
                            if (tab.id === 'docs') {
                                window.open('/v1/docs', '_blank');
                            } else {
                                setActiveTab(tab.id);
                            }
                        }}
                        style={{
                            padding: '1rem 1.5rem',
                            margin: '0.5rem 0',
                            borderRadius: '8px',
                            cursor: 'pointer',
                            display: 'flex',
                            alignItems: 'center',
                            gap: '12px',
                            transition: 'all 0.1s',
                            background: activeTab === tab.id ? 'var(--lw-black)' : 'transparent',
                            color: activeTab === tab.id ? 'var(--lw-bg)' : 'var(--lw-gray)',
                            border: activeTab === tab.id ? 'var(--lw-border)' : '1px solid transparent',
                            boxShadow: activeTab === tab.id ? 'var(--lw-shadow)' : 'none',
                        }}
                    >
                        <span style={{ fontSize: '1.2rem' }}>{tab.icon}</span>
                        <span style={{ fontWeight: 700, fontSize: '0.9rem', fontFamily: 'Outfit' }}>{tab.label.toUpperCase()}</span>
                    </div>
                ))}
            </nav>

            <div style={{ padding: '2rem', borderTop: 'var(--lw-border)', fontSize: '0.75rem', color: 'var(--lw-gray)', fontWeight: 600 }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                    <div style={{ width: '10px', height: '10px', borderRadius: '50%', background: 'var(--lw-green)', border: '1px solid #000' }}></div>
                    SYSTEM ONLINE
                </div>
            </div>
        </div>
    );
};

export default Sidebar;
