// Standard part operations shown at the bottom of every property window:
// Rotate · Size (scale) · Delete. Wired to the editor by the caller.
export interface PartActionsProps {
  onRotate: () => void;
  scale: number;
  onScale: (scale: number) => void;
  onDelete: () => void;
  /** False for the lab's primary board, which can't be deleted. */
  canDelete?: boolean;
}

export function PartActions({ onRotate, scale, onScale, onDelete, canDelete = true }: PartActionsProps) {
  return (
    <div className="flex shrink-0 items-center gap-3 border-t border-border bg-bg-elevated/30 px-3 py-2">
      <button
        type="button"
        onClick={onRotate}
        title="Rotate 90°"
        className="inline-flex items-center gap-1 rounded-md border border-border px-2 py-1 text-[11px] text-fg-secondary hover:bg-bg-elevated hover:text-fg-primary"
      >
        ↻ Rotate
      </button>
      <label className="flex items-center gap-1.5 text-[11px] text-fg-tertiary">
        Size
        <input
          type="range"
          min={0.5}
          max={2}
          step={0.1}
          value={scale}
          onChange={(e) => onScale(parseFloat(e.target.value))}
          className="w-20 accent-accent"
        />
      </label>
      <button
        type="button"
        onClick={onDelete}
        disabled={!canDelete}
        title={canDelete ? 'Delete' : "The lab's main board can't be deleted"}
        className="ml-auto rounded-md border border-red-500/40 px-2.5 py-1 text-[11px] text-red-300 hover:bg-red-500/10 disabled:cursor-not-allowed disabled:opacity-40"
      >
        Delete
      </button>
    </div>
  );
}
