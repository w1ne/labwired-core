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
*   **Local Mode**: The `labwired` CLI/DAP must be installed (included in the extension or available in PATH).
*   **Docker Mode**: Docker must be installed and running.

## Docker Support

LabWired supports running simulations, builds, and tests inside a Docker container. This ensures a consistent development environment and eliminates the need for manual toolchain setup.

### Configuration

To enable Docker mode, add the following to your VS Code settings:

```json
{
    "labwired.executionMode": "docker",
    "labwired.docker.image": "w1ne/labwired-dev:latest",
    "labwired.docker.autoPull": true
}
```

### Features

-   **Simulation in Docker**: The extensions runs the `labwired-dap` inside a container.
-   **Local Builds**: The extension uses your *local* toolchain (`cargo`, `make`) to build the firmware.
-   **Mirror Mounts**: The workspace is mounted to the exact same path inside the container, ensuring debug symbols match.
-   **Dashboard Access**: Port 9999 is automatically forwarded for the Live Dashboard.
