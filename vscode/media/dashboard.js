(function () {
    const vscode = acquireVsCodeApi();

    const mipsEl = document.getElementById('mips-value');
    const cyclesEl = document.getElementById('cycles-value');
    const pcEl = document.getElementById('pc-value');
    const statusTextEl = document.getElementById('status-text');
    const statusBadgeEl = document.getElementById('status-badge');
    const logEl = document.getElementById('log-output');
    const regGridEl = document.getElementById('register-grid');

    const mipsSparkline = new Sparkline('mips-sparkline', '#00a2ff');
    const cyclesSparkline = new Sparkline('cycles-sparkline', '#3fb950');

    let lastRegisters = {};

    window.addEventListener('message', event => {
        const message = event.data;
        switch (message.type) {
            case 'telemetry':
                updateDashboard(message.data);
                break;
        }
    });

    function Sparkline(canvasId, color) {
        const canvas = document.getElementById(canvasId);
        if (!canvas) return;
        const ctx = canvas.getContext('2d');
        const data = [];
        const maxPoints = 40;

        this.add = function (val) {
            data.push(val);
            if (data.length > maxPoints) data.shift();
            this.draw();
        };

        this.draw = function () {
            const width = canvas.width = canvas.parentElement.clientWidth;
            const height = canvas.height = canvas.parentElement.clientHeight;
            ctx.clearRect(0, 0, width, height);

            if (data.length < 2) return;

            const min = Math.min(...data);
            const max = Math.max(...data);
            const range = max - min || 1;

            ctx.beginPath();
            ctx.strokeStyle = color;
            ctx.lineWidth = 2;
            ctx.lineJoin = 'round';
            ctx.lineCap = 'round';

            for (let i = 0; i < data.length; i++) {
                const x = (i / (maxPoints - 1)) * width;
                const y = height - ((data[i] - min) / range) * (height * 0.8) - (height * 0.1);
                if (i === 0) ctx.moveTo(x, y);
                else ctx.lineTo(x, y);
            }
            ctx.stroke();

            // Gradient fill
            const grad = ctx.createLinearGradient(0, 0, 0, height);
            grad.addColorStop(0, color.replace(')', ', 0.2)').replace('rgb', 'rgba'));
            grad.addColorStop(1, 'transparent');
            ctx.lineTo((data.length - 1) / (maxPoints - 1) * width, height);
            ctx.lineTo(0, height);
            ctx.fillStyle = grad;
            ctx.fill();
        };
    }

    function updateDashboard(data) {
        if (mipsEl) mipsEl.textContent = data.mips.toFixed(2);
        if (cyclesEl) cyclesEl.textContent = data.cycles.toLocaleString();
        if (pcEl) pcEl.textContent = `0x${data.pc.toString(16).toUpperCase().padStart(8, '0')}`;

        if (data.status) {
            if (statusTextEl) statusTextEl.textContent = data.status;
            if (statusBadgeEl) {
                statusBadgeEl.textContent = data.status === 'Running' ? 'Live' : 'Stopped';
                statusBadgeEl.style.background = data.status === 'Running' ? 'rgba(63, 185, 80, 0.15)' : 'rgba(248, 81, 73, 0.15)';
                statusBadgeEl.style.color = data.status === 'Running' ? '#3fb950' : '#f85149';
                statusBadgeEl.style.borderColor = data.status === 'Running' ? 'rgba(63, 185, 80, 0.3)' : 'rgba(248, 81, 73, 0.3)';
            }
        }

        mipsSparkline.add(data.mips);
        cyclesSparkline.add(data.cycles % 1000); // Relative cycles for visualization

        if (regGridEl && data.registers) {
            renderRegisters(data.registers);
        }

        addLogEntry('info', `PC: 0x${data.pc.toString(16).toUpperCase()} | MIPS: ${data.mips.toFixed(2)}`);
    }

    function renderRegisters(registers) {
        const sortedKeys = Object.keys(registers).sort((a, b) => {
            const numA = parseInt(a.replace(/\D/g, ''));
            const numB = parseInt(b.replace(/\D/g, ''));
            if (!isNaN(numA) && !isNaN(numB)) return numA - numB;
            return a.localeCompare(b);
        });

        regGridEl.innerHTML = '';
        for (const key of sortedKeys) {
            const val = registers[key];
            const changed = lastRegisters[key] !== undefined && lastRegisters[key] !== val;

            const item = document.createElement('div');
            item.className = `reg-item ${changed ? 'changed' : ''}`;
            item.innerHTML = `
                <span class="reg-name">${key}</span>
                <span class="reg-value">0x${val.toString(16).toUpperCase()}</span>
            `;
            regGridEl.appendChild(item);
        }
        lastRegisters = { ...registers };
    }

    function addLogEntry(level, text) {
        const entry = document.createElement('div');
        entry.className = 'log-entry';
        const time = new Date().toLocaleTimeString([], { hour12: false });
        entry.innerHTML = `
            <span class="log-time">[${time}]</span>
            <span class="log-level-${level}">${level.toUpperCase()}</span>
            <span class="log-text">${text}</span>
        `;

        if (logEl) {
            logEl.appendChild(entry);
            logEl.scrollTop = logEl.scrollHeight;
            if (logEl.children.length > 100) {
                logEl.removeChild(logEl.firstChild);
            }
        }
    }
}());
