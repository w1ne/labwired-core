# Debugging Firmware in VS Code

LabWired includes a built-in GDB server that allows you to use standard debugging tools like VS Code to inspect your firmware's execution in real-time. It supports both ARM Cortex-M and RISC-V architectures.

## Prerequisites

1.  **VS Code** installed.
2.  **Extensions**:
    -   [Cortex-Debug](https://marketplace.visualstudio.com/items?itemName=marus25.cortex-debug) (Highly recommended for ARM)
    -   OR [C/C++](https://marketplace.visualstudio.com/items?itemName=ms-vscode.cpptools) (for standard GDB support, required for RISC-V)
3.  **Toolchain**:
    -   For ARM: `arm-none-eabi-gdb` must be in your PATH.
    -   For RISC-V: `riscv64-unknown-elf-gdb` (or similar) must be in your PATH.

## Project Configuration

LabWired comes with pre-configured `.vscode` files to get you started immediately.

### launch.json
This file defines how VS Code connects to the LabWired GDB server. It includes configurations for both architectures:
- **LabWired Debug (Cortex-Debug)**: Optimized for ARM Cortex-M.
- **LabWired Debug (GDB)**: Generic GDB configuration, works for both ARM and RISC-V.

### tasks.json
This file defines background tasks, such as starting the LabWired simulator in GDB mode before the debugger attaches.

## Step-by-Step Debugging

1.  **Build your Firmware**:
    Ensure your firmware is built with debug symbols.
    ```bash
    # For ARM
    cargo build --target thumbv7m-none-eabi
    # For RISC-V
    cargo build --target riscv32i-unknown-none-elf
    ```

2.  **Start Debugging**:
    -   Go to the "Run and Debug" view in VS Code (Ctrl+Shift+D).
    -   Select the appropriate configuration from the dropdown.
    -   Press **F5**.

3.  **What Happens Automatically**:
    -   VS Code runs the "Start LabWired GDB" task.
    -   LabWired starts and waits for a GDB connection on port `3333`.
    -   VS Code attaches to the GDB server, loads the symbols from your ELF, and hits the entry point (or `main`).

## Features Supported

-   **Breakpoints**: Set breakpoints directly in your Rust/C code.
-   **Step Over/Into/Out**: Step through your code instruction-by-instruction or line-by-line.
-   **Variables & Watch**: Inspect local and global variables.
-   **Memory View**: View raw memory at any address (Flash, RAM, or Peripherals).
-   **Registers**: View CPU registers (R0-R15 for ARM, x0-x31 for RISC-V).
-   **Peripheral Registers**: (With Cortex-Debug for ARM) Use an SVD file to view memory-mapped registers in a structured way.

## Troubleshooting

-   **"Connection Refused"**: Ensure LabWired started successfully. Check the "Terminal" tab in VS Code for any error messages from the "Start LabWired GDB" task.
-   **"Command not found"**: Ensure the appropriate GDB debugger (`arm-none-eabi-gdb` or `riscv64-unknown-elf-gdb`) is installed on your system.
-   **Instruction Tracing**: You can enable Instruction Tracing in `tasks.json` by adding `--trace` to the command for extra visibility in the terminal while debugging.
