import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { BoardConfig } from '../bundled-configs';
import { MobileRunView } from './MobileRunView';

const selectedBoard: BoardConfig = {
  boardId: 'stm32f103-blinky',
  chipId: 'stm32f103',
  name: 'Blinky',
  description: 'Test board',
  arch: 'ARM Cortex-M3',
  chipYaml: '',
  systemYaml: '',
  mcuComponentType: 'stm32-dev',
  kind: 'lab',
};

function renderMobileRunView(onPickLab = vi.fn()) {
  return render(
    <MobileRunView
      selectedBoard={selectedBoard}
      editorState={{
        diagram: { version: 1, board: 'stm32f103', parts: [], wires: [] },
        selectedIds: new Set(),
        wireInProgress: null,
        undoStack: [],
        redoStack: [],
      }}
      boardIoStates={{}}
      displayBuffers={{}}
      uartOutput=""
      onButtonToggle={vi.fn()}
      onAnalogChange={vi.fn()}
      onUpdateAttr={vi.fn()}
      ntcTemperatures={{}}
      onNtcChange={vi.fn()}
      simControls={<button type="button">Run</button>}
      onOpenProjects={vi.fn()}
      bridge={null}
      running={false}
      onPartAttrChange={vi.fn()}
      onPickLab={onPickLab}
    />,
  );
}

describe('MobileRunView lab picker', () => {
  it('lets mobile users switch to another example lab from the menu', async () => {
    const user = userEvent.setup();
    const onPickLab = vi.fn();
    renderMobileRunView(onPickLab);

    await user.click(screen.getByRole('button', { name: /open menu/i }));
    expect(screen.getByRole('list', { name: /example labs/i })).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: /adxl345 tilt/i }));

    expect(onPickLab).toHaveBeenCalledWith('adxl345-sensor-lab');
    expect(screen.queryByRole('button', { name: /adxl345 tilt/i })).not.toBeInTheDocument();
  });
});
