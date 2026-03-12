import { useState, useEffect } from 'react';
import { useAuth } from '@clerk/react';
import './App.css';
import Sidebar from './components/Sidebar';
import AssetCatalog from './components/AssetCatalog';
import UsageStats from './components/UsageStats';
import LandingPage from './components/LandingPage';
import AssetDetail from './components/AssetDetail';
import HealthMonitoring from './components/HealthMonitoring';
import RunDetail from './components/RunDetail';
import PrivateModels from './components/PrivateModels';

type Route = 'landing' | 'catalog' | 'usage' | 'asset' | 'health' | 'run' | 'private-models';

export function parseRoute(): { route: Route; assetId?: string; runId?: string } {
  const hash = window.location.hash.replace('#', '');
  if (hash.startsWith('/assets/')) {
    return { route: 'asset', assetId: hash.replace('/assets/', '') };
  }
  if (hash.startsWith('/runs/')) {
    return { route: 'run', runId: hash.replace('/runs/', '') };
  }
  if (hash === '/dashboard') return { route: 'usage' };
  if (hash === '/catalog') return { route: 'catalog' };
  if (hash === '/private-models') return { route: 'private-models' };
  if (hash === '/usage') return { route: 'usage' };
  if (hash === '/health') return { route: 'health' };
  return { route: 'landing' };
}

function App() {
  const { isSignedIn, isLoaded } = useAuth();
  const [routeState, setRouteState] = useState(parseRoute());

  useEffect(() => {
    const onHashChange = () => setRouteState(parseRoute());
    window.addEventListener('hashchange', onHashChange);
    return () => window.removeEventListener('hashchange', onHashChange);
  }, []);

  const navigate = (hash: string) => {
    window.location.hash = hash;
  };

  if (!isLoaded) return null;

  if (routeState.route === 'landing') {
    return <LandingPage onEnterDashboard={() => navigate('/dashboard')} />;
  }

  if (routeState.route === 'asset' && routeState.assetId) {
    return <AssetDetail id={routeState.assetId} onBack={() => navigate('/catalog')} />;
  }

  // Keep catalog public, but require auth for dashboard/usage/health/run/private-models routes.
  if (!isSignedIn && (routeState.route === 'usage' || routeState.route === 'health' || routeState.route === 'run' || routeState.route === 'private-models')) {
    return <LandingPage onEnterDashboard={() => navigate('/dashboard')} />;
  }

  if (routeState.route === 'run' && routeState.runId) {
    return <RunDetail runId={routeState.runId} onBack={() => navigate('/usage')} />;
  }

  if (routeState.route === 'catalog') {
    return (
      <div style={{ minHeight: '100vh', background: 'var(--lw-bg)', display: 'flex', flexDirection: 'column' }}>
        <nav style={{
          display: 'flex', justifyContent: 'space-between', alignItems: 'center',
          padding: '1.2rem 2.5rem', borderBottom: 'var(--lw-border)',
          background: 'var(--lw-bg)', position: 'sticky', top: 0, zIndex: 100,
        }}>
          <div style={{ fontFamily: 'Outfit', fontWeight: 900, fontSize: '1.3rem', letterSpacing: '-0.03em', cursor: 'pointer' }} onClick={() => navigate('/')}>
            LABWIRED <span style={{ color: 'var(--lw-pink)' }}>FOUNDRY</span>
          </div>
          <div style={{ display: 'flex', gap: '1rem', alignItems: 'center' }}>
            <button onClick={() => navigate('/dashboard')} className="secondary" style={{ padding: '0.5rem 1rem' }}>Dashboard →</button>
          </div>
        </nav>
        <div style={{ width: '100%' }}>
          <AssetCatalog onSelectAsset={(id) => navigate(`/assets/${id}`)} />
        </div>
      </div>
    );
  }

  return (
    <div style={{ display: 'flex', width: '100vw', minHeight: '100vh', background: 'var(--lw-bg-alt)' }}>
      <Sidebar activeTab={routeState.route === 'run' ? 'usage' : routeState.route} setActiveTab={(tab: string) => navigate(`/${tab}`)} />
      <main style={{ flex: 1, display: 'flex', overflowY: 'auto' }}>
        {routeState.route === 'private-models' && <PrivateModels />}
        {routeState.route === 'usage' && <UsageStats />}
        {routeState.route === 'health' && <HealthMonitoring />}
      </main>
      <div style={{
        position: 'fixed', top: '5%', right: '5%',
        width: '300px', height: '300px',
        background: 'rgba(232, 62, 140, 0.02)',
        filter: 'blur(80px)', borderRadius: '50%', zIndex: -1,
      }} />
    </div>
  );
}

export default App;
