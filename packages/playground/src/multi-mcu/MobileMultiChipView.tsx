// Mobile-first multi-MCU view. Two screens:
//   1. Chip canvas (default) — vertical list of MCU tiles with the
//      Wokwi-style thumbnail, board name, status LED, and a
//      "Properties" button. Tap a tile to focus that MCU; tap
//      Properties to push the full-screen properties window.
//   2. Properties window — full-screen modal with Serial / Registers
//      / Trace / Memory / Source / YAML tabs for the active MCU.
//
// Keeps the canvas simple (a stack of cards) and lets each chip's
// properties take the full viewport when actively inspected —
// matches the user's "full window properties on mobile" model.
import { useState, type ReactNode } from 'react';
import { useChips } from './ChipsProvider';
import type { ChipSession } from './ChipsProvider';
import { McuThumb } from './McuThumb';
import './mobile-chip.css';

export interface MobileMultiChipViewProps {
  // Slot for the active MCU's properties content (tabs + body).
  // Provided by App.tsx as the existing dev-drawer rendering for
  // the active chip so we don't duplicate tab logic.
  propertiesContent?: ReactNode;
  // Sim controls slot (Run / Pause) for the active MCU.
  simControls?: ReactNode;
  // UART output of the active chip, shown inline on the chip card.
  uartPreview?: string;
  running?: boolean;
  cyclesActive?: number;
  // Whether the active chip currently has firmware available to run
  // (source compiled or bridge already instantiated). Drives the
  // per-chip Run button visibility.
  canRun?: boolean;
  onRun?: () => void;
  onPause?: () => void;
  // Renders the ⌘K command palette overlay; receives open/close
  // callbacks so the mobile header search button can drive it.
  renderCommandPalette?: (
    open: boolean,
    close: () => void,
    open_: () => void,
  ) => ReactNode;
}

export function MobileMultiChipView({
  propertiesContent,
  uartPreview,
  running,
  cyclesActive,
  canRun,
  onRun,
  onPause,
  renderCommandPalette,
}: MobileMultiChipViewProps) {
  const {
    order,
    sessions,
    activeChipId,
    setActiveChipId,
    removeChip,
    propertiesOpen: propsOpen,
    setPropertiesOpen: setPropsOpen,
  } = useChips();
  const [commandOpen, setCommandOpen] = useState(false);

  return (
    <div className="lw-mob">
      <header className="lw-mob-header">
        <span className="lw-mob-logo">LabWired</span>
        <span className="lw-mob-sub">{order.length} MCU{order.length === 1 ? '' : 's'}</span>
        <button
          type="button"
          className="lw-mob-search"
          aria-label="Search components & actions"
          onClick={() => setCommandOpen(true)}
        >
          🔍
        </button>
      </header>

      <main className="lw-mob-canvas">
        {order.map((chipId) => {
          const session = sessions[chipId];
          if (!session) return null;
          const isActive = chipId === activeChipId;
          return (
            <MobileChipCard
              key={chipId}
              session={session}
              isActive={isActive}
              running={isActive && !!running}
              cycles={isActive ? cyclesActive : undefined}
              uartPreview={isActive ? uartPreview : undefined}
              canRun={isActive ? !!canRun : !!session.source || !!session.bridge}
              onRun={
                isActive
                  ? onRun
                  : () => {
                      setActiveChipId(chipId);
                      // Run kicks in once App.tsx restores this
                      // chip's source/bridge — the next frame.
                      setTimeout(() => onRun?.(), 60);
                    }
              }
              onPause={isActive ? onPause : undefined}
              onFocus={() => setActiveChipId(chipId)}
              onOpenProperties={() => {
                setActiveChipId(chipId);
                setPropsOpen(true);
              }}
              onRemove={chipId === 'chip-default' ? undefined : () => removeChip(chipId)}
            />
          );
        })}
        <button type="button" className="lw-mob-add" onClick={() => setCommandOpen(true)}>
          + add component
        </button>
      </main>

      {renderCommandPalette?.(commandOpen, () => setCommandOpen(false), () => setCommandOpen(true))}

      {propsOpen && sessions[activeChipId] && (
        <div className="lw-mob-props" role="dialog" aria-modal="true">
          <header className="lw-mob-props-header">
            <button
              type="button"
              className="lw-mob-back"
              onClick={() => setPropsOpen(false)}
              aria-label="Back to chip canvas"
            >
              ←
            </button>
            <span className="lw-mob-props-title">
              {sessions[activeChipId]!.chipId} · {sessions[activeChipId]!.board.name}
            </span>
          </header>
          <div className="lw-mob-props-body">{propertiesContent}</div>
        </div>
      )}
    </div>
  );
}

interface MobileChipCardProps {
  session: ChipSession;
  isActive: boolean;
  running: boolean;
  cycles?: number;
  uartPreview?: string;
  canRun: boolean;
  onRun?: () => void;
  onPause?: () => void;
  onFocus: () => void;
  onOpenProperties: () => void;
  onRemove?: () => void;
}

function MobileChipCard({
  session,
  isActive,
  running,
  cycles,
  uartPreview,
  canRun,
  onRun,
  onPause,
  onFocus,
  onOpenProperties,
  onRemove,
}: MobileChipCardProps) {
  return (
    <article
      className="lw-mob-card"
      data-active={isActive ? 'true' : 'false'}
      onClick={isActive ? undefined : onFocus}
    >
      <div className="lw-mob-card-head">
        <div className="lw-mob-card-thumb">
          <McuThumb session={session} width={86} height={56} />
        </div>
        <div className="lw-mob-card-meta">
          <div className="lw-mob-card-id">{session.chipId}</div>
          <div className="lw-mob-card-board">{session.board.name}</div>
          <div className="lw-mob-card-status">
            <span
              className="lw-mob-card-led"
              data-state={running ? 'running' : session.bridge ? 'paused' : 'idle'}
            />
            {running
              ? `${(cycles ?? 0).toLocaleString()} cycles`
              : session.bridge
                ? 'paused'
                : 'no firmware'}
          </div>
        </div>
        {onRemove && (
          <button
            type="button"
            className="lw-mob-card-remove"
            aria-label={`Remove ${session.chipId}`}
            onClick={(e) => {
              e.stopPropagation();
              onRemove();
            }}
          >
            ×
          </button>
        )}
      </div>

      <div className="lw-mob-card-actions">
        {canRun && (
          <button
            type="button"
            className="lw-mob-card-run"
            data-running={running ? 'true' : 'false'}
            onClick={(e) => {
              e.stopPropagation();
              if (running) onPause?.();
              else onRun?.();
            }}
          >
            {running ? '⏸ Pause' : '▶ Run'}
          </button>
        )}
        <button
          type="button"
          className="lw-mob-card-props"
          onClick={(e) => {
            e.stopPropagation();
            onOpenProperties();
          }}
        >
          Properties
        </button>
      </div>
      {isActive && uartPreview && (
        <pre className="lw-mob-card-uart" aria-label="Recent serial output">
          {uartPreview.trimEnd().split('\n').slice(-3).join('\n')}
        </pre>
      )}
    </article>
  );
}
