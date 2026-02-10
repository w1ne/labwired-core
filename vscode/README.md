# LabWired VS Code Extension

Official VS Code extension for the **LabWired Firmware Simulation Platform**.

## Features
*   **Timeline View**: Visualize execution history (PC, Interrupts) over time.
*   **Register Inspector**: View and modify peripheral registers with bit-field expansion.
*   **Memory View**: Inspect and edit memory in Hex/ASCII.
*   **Debug Adapter**: Connects to the `labwired` GDB/DAP server.

## Development

1.  **Install Dependencies**:
    ```bash
    npm install
    ```

2.  **Compile**:
    ```bash
    npm run compile
    ```

3.  **Run**:
    Open this folder in VS Code and press `F5`.

## Requirements
*   The `labwired` CLI must be installed and available in your implementation plan.
