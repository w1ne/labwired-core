import { useState } from 'react'
import './App.css'
import Sidebar from './components/Sidebar'
import AssetCatalog from './components/AssetCatalog'
import UsageStats from './components/UsageStats'

function App() {
  const [activeTab, setActiveTab] = useState('catalog')

  return (
    <div style={{ display: 'flex', width: '100vw', minHeight: '100vh', background: 'var(--lw-bg-alt)' }}>
      <Sidebar activeTab={activeTab} setActiveTab={setActiveTab} />

      <main style={{ flex: 1, display: 'flex', overflowY: 'auto' }}>
        {activeTab === 'catalog' && <AssetCatalog />}
        {activeTab === 'usage' && <UsageStats />}
      </main>

      {/* Decorative Brand Elements */}
      <div style={{
        position: 'fixed',
        top: '5%',
        right: '5%',
        width: '300px',
        height: '300px',
        background: 'rgba(232, 62, 140, 0.02)',
        filter: 'blur(80px)',
        borderRadius: '50%',
        zIndex: -1
      }}></div>
    </div>
  )
}

export default App
