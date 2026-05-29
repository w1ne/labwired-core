// Phase 1 of the canvas refactor: a custom tldraw shape that renders
// arbitrary React children (today's StudioShell) inside its bounds. In
// Phase 2 the body becomes per-chip tabs (Code | Registers | Peripherals
// | Logs) driven by useChipSession.
import { createContext, useContext, type ReactNode } from 'react';
import {
  HTMLContainer,
  Rectangle2d,
  ShapeUtil,
  T,
  type TLBaseShape,
} from 'tldraw';

export interface ChipShapeProps {
  w: number;
  h: number;
  chipId: string;
}

export type ChipShape = TLBaseShape<'chip', ChipShapeProps>;

/// Augment tldraw's global shape registry so `TLShape` includes our
/// custom `'chip'` type — required for `ShapeUtil<ChipShape>` to
/// satisfy its constraint without `any` casts.
declare module '@tldraw/tlschema' {
  interface TLGlobalShapePropsMap {
    chip: ChipShapeProps;
  }
}

/// Bridges React children (the full StudioShell tree) into the body of a
/// tldraw shape without leaking through shape props (which must be
/// JSON-serializable for the canvas snapshot store).
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
    return { w: 1024, h: 768, chipId: 'chip-0' };
  }

  override getGeometry(shape: ChipShape) {
    return new Rectangle2d({
      width: shape.props.w,
      height: shape.props.h,
      isFilled: true,
    });
  }

  // Phase 2a: chip is draggable but not resizable yet — resize comes
  // in Phase 2b once multi-chip layouts need it.
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
    // eslint-disable-next-line react-hooks/rules-of-hooks
    const children = useContext(ChipChildrenContext);
    return (
      <HTMLContainer
        id={shape.id}
        style={{
          width: shape.props.w,
          height: shape.props.h,
          // pointerEvents: 'all' — embedded StudioShell takes clicks;
          // tldraw still gets drag events from the shape outline (the
          // 4px transparent edge), which is enough to move the chip.
          pointerEvents: 'all',
          overflow: 'hidden',
          background: '#0a0a0f',
          borderRadius: 12,
          border: '1px solid rgba(255, 255, 255, 0.08)',
          boxShadow: '0 24px 64px rgba(0, 0, 0, 0.45)',
          display: 'flex',
          flexDirection: 'column',
        }}
      >
        <div
          // Drag handle: a 28px-tall header strip at the top. The chip
          // body below is interactive (Monaco, palette, etc.), so the
          // strip is the reliable place to grab and reposition the chip.
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
            // Forward pointer events to tldraw so the strip is the drag
            // affordance even though the chip body is interactive.
            pointerEvents: 'none',
            userSelect: 'none',
          }}
        >
          <span style={{ opacity: 0.5 }}>●●●</span>
          <span style={{ marginLeft: 12 }}>{shape.props.chipId}</span>
        </div>
        <div style={{ flex: 1, minHeight: 0, overflow: 'hidden' }}>{children}</div>
      </HTMLContainer>
    );
  }

  override getIndicatorPath(shape: ChipShape) {
    const path = new Path2D();
    path.rect(0, 0, shape.props.w, shape.props.h);
    return path;
  }
}
