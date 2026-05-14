import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import '../styles/tokens.css';
import '../styles/tailwind.css';
import { LegacyNotice } from './App.legacy';

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <LegacyNotice />
  </StrictMode>,
);
