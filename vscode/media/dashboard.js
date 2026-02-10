(function () {
    const vscode = acquireVsCodeApi();

    const mipsEl = document.getElementById('mips-value');
    const cyclesEl = document.getElementById('cycles-value');
    const pcEl = document.getElementById('pc-value');
    const logEl = document.getElementById('log-output');
    const regGridEl = document.getElementById('register-grid');

    window.addEventListener('message', event => {
        const message = event.data;
        switch (message.type) {
            case 'telemetry':
                updateDashboard(message.data);
                break;
        }
    });

    function updateDashboard(data) {
        if (mipsEl) mipsEl.textContent = data.mips.toFixed(2);
        if (cyclesEl) cyclesEl.textContent = data.cycles.toLocaleString();
        if (pcEl) pcEl.textContent = `0x${data.pc.toString(16).toUpperCase().padStart(8, '0')}`;

        if (regGridEl && data.registers) {
            regGridEl.innerHTML = '';
            // Sort keys to maintain stable positions if names change
            const sortedKeys = Object.keys(data.registers).sort((a, b) => {
                // Numeric sort for R0, R1...
                const numA = parseInt(a.replace(/\D/g, ''));
                const numB = parseInt(b.replace(/\D/g, ''));
                if (!isNaN(numA) && !isNaN(numB)) return numA - numB;
                return a.localeCompare(b);
            });

            for (const key of sortedKeys) {
                const item = document.createElement('div');
                item.className = 'reg-item';
                item.innerHTML = `
                    <span class="reg-name">${key}</span>
                    <span class="reg-value">0x${data.registers[key].toString(16).toUpperCase()}</span>
                `;
                regGridEl.appendChild(item);
            }
        }

        // Add a log entry for significant jumps (e.g. PC move)
        // For now, let's just log every update to show life
        const entry = document.createElement('div');
        entry.textContent = `[${new Date().toLocaleTimeString()}] PC: 0x${data.pc.toString(16).toUpperCase()} | MIPS: ${data.mips.toFixed(2)}`;
        if (logEl) {
            logEl.appendChild(entry);
            logEl.scrollTop = logEl.scrollHeight;
            if (logEl.children.length > 50) {
                logEl.removeChild(logEl.firstChild);
            }
        }
    }
}());
