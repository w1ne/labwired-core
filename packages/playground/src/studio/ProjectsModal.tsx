// ProjectsModal — list / load / save / delete the signed-in user's projects.
// All storage is server-side via /v1/projects/* (see useProjects).
import { useState } from 'react';
import { useUser } from '@clerk/clerk-react';
import { useProjects, type ProjectRecord, type ProjectSummary } from './useProjects';

export interface ProjectsModalProps {
  open: boolean;
  onClose: () => void;
  /** Current canvas state — what "Save current as new" uploads. */
  currentBoardId: string;
  currentDiagramJson: string;
  currentSourceCode: string | null;
  /** Project the user is currently editing (set after a Save / Load). */
  activeProjectId: string | null;
  activeProjectName: string | null;
  /** Called when user picks a project from the list to load into the canvas. */
  onLoad: (project: ProjectRecord) => void;
  /** Called when a new project is created (so the parent can update activeProjectId/Name). */
  onCreated: (project: ProjectRecord) => void;
  /** Called when the active project is updated in-place (Save). */
  onSaved: (project: ProjectRecord) => void;
}

function formatRelative(ms: number): string {
  const delta = Date.now() - ms;
  if (delta < 60_000) return 'just now';
  if (delta < 3_600_000) return `${Math.floor(delta / 60_000)}m ago`;
  if (delta < 86_400_000) return `${Math.floor(delta / 3_600_000)}h ago`;
  return new Date(ms).toLocaleDateString();
}

export function ProjectsModal({
  open,
  onClose,
  currentBoardId,
  currentDiagramJson,
  currentSourceCode,
  activeProjectId,
  activeProjectName,
  onLoad,
  onCreated,
  onSaved,
}: ProjectsModalProps) {
  const { isSignedIn } = useUser();
  const projects = useProjects(open && !!isSignedIn);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [newName, setNewName] = useState('');

  if (!open) return null;

  async function handleSaveCurrent(asNew: boolean) {
    const name = asNew || !activeProjectId ? newName.trim() : (activeProjectName ?? newName.trim());
    if (!name) return;
    setBusyId('__save__');
    try {
      const saved = asNew || !activeProjectId
        ? await projects.create({
            name,
            boardId: currentBoardId,
            diagramJson: currentDiagramJson,
            sourceCode: currentSourceCode,
          })
        : await projects.update(activeProjectId, {
            name,
            boardId: currentBoardId,
            diagramJson: currentDiagramJson,
            sourceCode: currentSourceCode,
          });
      if (saved) {
        setNewName('');
        if (asNew || !activeProjectId) onCreated(saved);
        else onSaved(saved);
      }
    } finally {
      setBusyId(null);
    }
  }

  async function handleLoad(summary: ProjectSummary) {
    setBusyId(summary.id);
    try {
      const full = await projects.load(summary.id);
      if (full) {
        onLoad(full);
        onClose();
      }
    } finally {
      setBusyId(null);
    }
  }

  async function handleDelete(summary: ProjectSummary) {
    if (!confirm(`Delete "${summary.name}"? This cannot be undone.`)) return;
    setBusyId(summary.id);
    try {
      await projects.remove(summary.id);
    } finally {
      setBusyId(null);
    }
  }

  return (
    <div className="projects-modal-backdrop" onClick={onClose}>
      <div className="projects-modal" onClick={(e) => e.stopPropagation()}>
        <div className="projects-modal-header">
          <h2>My Projects</h2>
          <button className="projects-modal-close" onClick={onClose} aria-label="Close">×</button>
        </div>

        {!isSignedIn ? (
          <div className="projects-modal-empty">
            <p>Sign in to save and sync your projects across devices.</p>
          </div>
        ) : (
          <>
            <div className="projects-modal-save">
              <input
                type="text"
                className="projects-modal-input"
                placeholder={activeProjectId ? 'Save as new project — enter name' : 'Project name'}
                value={newName}
                onChange={(e) => setNewName(e.target.value)}
                disabled={busyId === '__save__'}
              />
              <button
                className="projects-modal-btn primary"
                onClick={() => handleSaveCurrent(true)}
                disabled={!newName.trim() || busyId === '__save__'}
              >
                {busyId === '__save__' ? 'Saving…' : 'Save as new'}
              </button>
              {activeProjectId && (
                <button
                  className="projects-modal-btn"
                  onClick={() => handleSaveCurrent(false)}
                  disabled={busyId === '__save__'}
                  title={`Overwrite "${activeProjectName}"`}
                >
                  Save to "{activeProjectName}"
                </button>
              )}
            </div>

            {projects.status === 'loading' && (
              <div className="projects-modal-empty">Loading projects…</div>
            )}
            {projects.status === 'error' && (
              <div className="projects-modal-error">
                {projects.error}
                {projects.error?.includes('Not found') && (
                  <p style={{ marginTop: 8, fontSize: 11 }}>
                    The projects API isn't deployed yet. Backend code is in
                    place; the API Worker deploy workflow should run after
                    merging the backend change to <code>main</code>. Confirm
                    <code>KV_PROJECTS</code> exists if this persists.
                  </p>
                )}
              </div>
            )}
            {projects.status === 'ok' && projects.list.length === 0 && (
              <div className="projects-modal-empty">
                No saved projects yet. Wire up a circuit and save it above.
              </div>
            )}
            {projects.status === 'ok' && projects.list.length > 0 && (
              <ul className="projects-modal-list">
                {projects.list.map((p) => (
                  <li
                    key={p.id}
                    className={`projects-modal-item ${p.id === activeProjectId ? 'active' : ''}`}
                  >
                    <div className="projects-modal-item-info">
                      <div className="projects-modal-item-name">
                        {p.name}
                        {p.id === activeProjectId && <span className="badge badge-demo">Open</span>}
                      </div>
                      <div className="projects-modal-item-meta">
                        <span>{p.board_id}</span>
                        <span>updated {formatRelative(p.updated_at)}</span>
                      </div>
                    </div>
                    <div className="projects-modal-item-actions">
                      <button
                        className="projects-modal-btn"
                        onClick={() => handleLoad(p)}
                        disabled={busyId !== null}
                      >
                        {busyId === p.id ? 'Loading…' : 'Load'}
                      </button>
                      <button
                        className="projects-modal-btn danger"
                        onClick={() => handleDelete(p)}
                        disabled={busyId !== null}
                      >
                        Delete
                      </button>
                    </div>
                  </li>
                ))}
              </ul>
            )}
          </>
        )}
      </div>
    </div>
  );
}
