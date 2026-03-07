const PrivateModels = () => {
    return (
        <div style={{ padding: '2rem', flex: 1 }}>
            <header style={{ marginBottom: '3rem' }}>
                <h1 style={{ fontSize: '3rem', color: 'var(--lw-black)' }}>PRIVATE MODELS</h1>
                <p style={{ color: 'var(--lw-gray)', marginTop: '0.5rem', maxWidth: '600px', fontSize: '1.1rem' }}>
                    Dashboard catalog for your private models and advanced module combinations (e.g. sensor + core).
                </p>
            </header>
            <div style={{ padding: '3rem', border: '2px dashed var(--lw-border)', borderRadius: '12px', textAlign: 'center' }}>
                <div style={{ fontSize: '2rem', marginBottom: '1rem' }}>🏗️</div>
                <h3 style={{ fontSize: '1.2rem', color: 'var(--lw-black)', marginBottom: '0.5rem' }}>Under Construction</h3>
                <p style={{ color: 'var(--lw-gray)', fontSize: '0.9rem' }}>You will be able to manage your private IP and custom combinations here soon.</p>
            </div>
        </div>
    );
};

export default PrivateModels;
