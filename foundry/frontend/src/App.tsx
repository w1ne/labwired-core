import { useState, useEffect } from 'react';
import './App.css';
import Sidebar from './components/Sidebar';
import AssetCatalog from './components/AssetCatalog';
import UsageStats from './components/UsageStats';
import LandingPage from './components/LandingPage';
import AssetDetail from './components/AssetDetail';
// import HealthMonitoring from './components/HealthMonitoring';

type Route = 'landing' | 'catalog' | 'usage' | 'asset' | 'health';

function parseRoute(): { route: Route; assetId?: string } {
  const hash = window.location.hash.replace('#', '');
  if (hash.startsWith('/assets/')) {
    return { route: 'asset', assetId: hash.replace('/assets/', '') };
  }
  if (hash === '/dashboard' || hash === '/catalog') return { route: 'catalog' };
  if (hash === '/usage') return { route: 'usage' };
  if (hash === '/health') return { route: 'health' };
  return { route: 'landing' };
}

function App() {
  const [routeState, setRouteState] = useState(parseRoute());

  useEffect(() => {
    const onHashChange = () => setRouteState(parseRoute());
    window.addEventListener('hashchange', onHashChange);
    return () => window.removeEventListener('hashchange', onHashChange);
  }, []);

  const navigate = (hash: string) => {
    window.location.hash = hash;
  };

  if (routeState.route === 'landing') {
    return <LandingPage onEnterDashboard={() => navigate('/dashboard')} />;
  }

  if (routeState.route === 'asset' && routeState.assetId) {
    return <AssetDetail id={routeState.assetId} onBack={() => navigate('/dashboard')} />;
  }

  return (
    <div style={{ display: 'flex', width: '100vw', minHeight: '100vh', background: 'var(--lw-bg-alt)' }}>
      <Sidebar activeTab={routeState.route} setActiveTab={(tab: string) => navigate(`/${tab}`)} />
      <main style={{ flex: 1, display: 'flex', overflowY: 'auto' }}>
        {routeState.route === 'catalog' && <AssetCatalog onSelectAsset={(id) => navigate(`/assets/${id}`)} />}
        {routeState.route === 'usage' && <UsageStats />}
        {/* {routeState.route === 'health' && <HealthMonitoring />} */}
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
