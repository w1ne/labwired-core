import * as assert from 'assert';
import * as vscode from 'vscode';
import { SimulatorManager, SimulatorStatus } from '../../simulatorManager';
import { EventEmitter } from 'events';

suite('SimulatorManager Test Suite', () => {
    let mockOutputChannel: any;
    let mockCpModule: any;

    setup(() => {
        mockOutputChannel = {
            appendLine: (line: string) => console.log(line),
            append: (data: string) => console.log(data)
        };

        mockCpModule = {
            spawn: (bin: string, args: string[]) => {
                const process = new EventEmitter() as any;
                process.stdout = new EventEmitter();
                process.stderr = new EventEmitter();
                process.kill = () => process.emit('close', 0);
                return process;
            }
        };
    });

    test('SimulatorManager should start and change status', async () => {
        const mgr = new SimulatorManager(mockOutputChannel, mockCpModule);
        assert.strictEqual(mgr.status, SimulatorStatus.Stopped);

        await mgr.start('test-bin', []);
        assert.strictEqual(mgr.status, SimulatorStatus.Running);
    });

    test('SimulatorManager should stop and change status', async () => {
        const mgr = new SimulatorManager(mockOutputChannel, mockCpModule);
        await mgr.start('test-bin', []);
        mgr.stop();
        assert.strictEqual(mgr.status, SimulatorStatus.Stopped);
    });

    test('SimulatorManager should handle process errors', async () => {
        const mgr = new SimulatorManager(mockOutputChannel, {
            spawn: () => {
                const process = new EventEmitter() as any;
                // Emit error after a tick
                setTimeout(() => process.emit('error', new Error('test error')), 0);
                return process;
            }
        });

        await mgr.start('test-bin', []);
        // Wait for error event
        await new Promise(resolve => setTimeout(resolve, 10));
        assert.strictEqual(mgr.status, SimulatorStatus.Error);
    });
});
