import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import '../styles/tokens.css';
import '../styles/tailwind.css';
import '../ci/ci-light.css';
import { ValidationLanding } from './ValidationLanding';

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ValidationLanding />
  </StrictMode>,
);
