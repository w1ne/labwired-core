import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import '../styles/tokens.css';
import '../styles/tailwind.css';
import { CiLanding } from './CiLanding';

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <CiLanding />
  </StrictMode>,
);
