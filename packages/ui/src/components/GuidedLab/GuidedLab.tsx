import type { ReactNode } from 'react';

export interface GuidedLabStep {
  id: string;
  label: string;
  status: 'done' | 'active' | 'pending';
}

export interface GuidedLabProps {
  title: string;
  subtitle: string;
  steps: GuidedLabStep[];
  stage: ReactNode;
  sensor: ReactNode;
  serial: ReactNode;
  advanced: ReactNode;
  advancedOpen: boolean;
  onToggleAdvanced: () => void;
}

export function GuidedLab({
  title,
  subtitle,
  steps,
  stage,
  sensor,
  serial,
  advanced,
  advancedOpen,
  onToggleAdvanced,
}: GuidedLabProps) {
  return (
    <section className="guided-lab">
      <aside className="guided-lab-rail">
        <div>
          <h2>{title}</h2>
          <p>{subtitle}</p>
        </div>
        <ol>
          {steps.map((step) => (
            <li key={step.id} className={`guided-step ${step.status}`}>
              <span>{step.label}</span>
            </li>
          ))}
        </ol>
      </aside>
      <main className="guided-lab-stage">{stage}</main>
      <aside className="guided-lab-inspector">
        {sensor}
        {serial}
        <button className="guided-advanced-toggle" type="button" onClick={onToggleAdvanced}>
          {advancedOpen ? 'Hide Advanced' : 'Show Advanced'}
        </button>
      </aside>
      {advancedOpen && <div className="guided-lab-advanced">{advanced}</div>}
    </section>
  );
}
