// Phase 1+2a+2b ChipShape:
//   - Phase 1 created the shape type.
//   - Phase 2a turned the canvas interactive.
//   - Phase 2b makes the shape branch on activeChipId: the active chip
//     renders the StudioShell (via ChipChildrenContext); inactive chips
//     render a compact ChipCard that focuses the chip on click.
//
// Layout: active chip is a full StudioShell-sized panel; inactive chips
// are small cards. Sizes are persisted in the shape so users can resize
// when Phase 3 lands.
import { createContext, useContext, type ReactNode } from 'react';
import {
  HTMLContainer,
  Rectangle2d,
  ShapeUtil,
  T,
  type TLBaseShape,
} from 'tldraw';
import { useChips, useChipSession } from './ChipSession';
import { ChipCard } from './ChipCard';

export interface ChipShapeProps {
  w: number;
  h: number;
  chipId: string;
}

export type ChipShape = TLBaseShape<'chip', ChipShapeProps>;

declare module '@tldraw/tlschema' {
  interface TLGlobalShapePropsMap {
    chip: ChipShapeProps;
  }
}

/// React content injected into the *active* chip's body. Inactive chips
/// render <ChipCard> instead, so they don't need this content stream.
const ChipChildrenContext = createContext<ReactNode>(null);

export function ChipChildrenProvider({
  children,
  content,
}: {
  children: ReactNode;
  content: ReactNode;
}) {
  return <ChipChildrenContext.Provider value={content}>{children}</ChipChildrenContext.Provider>;
}

export class ChipShapeUtil extends ShapeUtil<ChipShape> {
  static override type = 'chip' as const;
  static override props = {
    w: T.number,
    h: T.number,
    chipId: T.string,
  };

  override getDefaultProps(): ChipShape['props'] {
    return { w: 1280, h: 800, chipId: 'chip-0' };
  }

  override getGeometry(shape: ChipShape) {
    return new Rectangle2d({
      width: shape.props.w,
      height: shape.props.h,
      isFilled: true,
    });
  }

  override canResize() {
    return false;
  }
  override canEdit() {
    return false;
  }
  override hideRotateHandle() {
    return true;
  }
  override hideResizeHandles() {
    return true;
  }

  override component(shape: ChipShape) {
    return <ChipShapeBody shape={shape} />;
  }

  override getIndicatorPath(shape: ChipShape) {
    const path = new Path2D();
    path.rect(0, 0, shape.props.w, shape.props.h);
    return path;
  }
}

function ChipShapeBody({ shape }: { shape: ChipShape }) {
  const children = useContext(ChipChildrenContext);
  const chips = useChips();
  const session = useChipSession(shape.props.chipId);
  const isActive = chips.activeChipId === shape.props.chipId;

  return (
    <HTMLContainer
      id={shape.id}
      style={{
        width: shape.props.w,
        height: shape.props.h,
        pointerEvents: 'all',
        overflow: 'hidden',
        background: '#0a0a0f',
        borderRadius: 12,
        border: isActive
          ? '1px solid rgba(232, 62, 140, 0.4)'
          : '1px solid rgba(255, 255, 255, 0.08)',
        boxShadow: isActive
          ? '0 24px 64px rgba(232, 62, 140, 0.18)'
          : '0 12px 32px rgba(0, 0, 0, 0.4)',
        display: 'flex',
        flexDirection: 'column',
      }}
    >
      <div
        style={{
          height: 28,
          flexShrink: 0,
          display: 'flex',
          alignItems: 'center',
          padding: '0 12px',
          background: 'rgba(255, 255, 255, 0.04)',
          borderBottom: '1px solid rgba(255, 255, 255, 0.06)',
          color: 'rgba(255, 255, 255, 0.6)',
          fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
          fontSize: 11,
          letterSpacing: 0.2,
          pointerEvents: 'none',
          userSelect: 'none',
        }}
      >
        <span style={{ opacity: 0.5 }}>●●●</span>
        <span style={{ marginLeft: 12 }}>{shape.props.chipId}</span>
        {!isActive && session?.bridge && (
          <span style={{ marginLeft: 'auto', opacity: 0.5 }}>● running</span>
        )}
      </div>
      <div style={{ flex: 1, minHeight: 0, overflow: 'hidden' }}>
        {isActive ? children : session ? <ChipCard session={session} /> : null}
      </div>
    </HTMLContainer>
  );
}
