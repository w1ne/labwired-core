import * as assert from 'assert';
import * as vscode from 'vscode';
import { LabwiredDashboardProvider } from '../../dashboard';
import { PeripheralProvider } from '../../peripheralProvider';
import { RTOSProvider } from '../../rtosProvider';

suite('Providers Test Suite', () => {
    vscode.window.showInformationMessage('Start all tests.');

    test('PeripheralProvider should handle missing debug session', async () => {
        const provider = new PeripheralProvider();
        const children = await provider.getChildren();
        assert.strictEqual(children.length, 0);
    });

    test('PeripheralProvider refresh should fire event', (done) => {
        const provider = new PeripheralProvider();
        provider.onDidChangeTreeData(() => {
            done();
        });
        provider.refresh();
    });

    test('RTOSProvider should handle missing debug session', async () => {
        const provider = new RTOSProvider();
        const children = await provider.getChildren();
        assert.strictEqual(children.length, 0);
    });

    test('RTOSProvider should parse tasks from customRequest', async () => {
        const provider = new RTOSProvider();
        const mockSession = {
            customRequest: (command: string) => {
                if (command === 'readRTOSState') {
                    return Promise.resolve({
                        tasks: [
                            { name: 'TestTask', state: 'Running', stackUsage: 50, priority: 1 }
                        ]
                    });
                }
                return Promise.reject('Unknown command');
            }
        } as any as vscode.DebugSession;

        const children = await provider.getChildren(undefined, mockSession);
        assert.strictEqual(children.length, 1);
        assert.strictEqual(children[0].label, 'TestTask');
        assert.strictEqual(children[0].description, 'Running');
    });

    test('PeripheralProvider should parse peripherals and registers', async () => {
        const provider = new PeripheralProvider();
        const mockSession = {
            customRequest: (command: string) => {
                if (command === 'readPeripherals') {
                    return Promise.resolve({
                        peripherals: [
                            {
                                name: 'UART1',
                                base: 0x4000C000,
                                size: 0x1000,
                                registers: [
                                    { name: 'DR', offset: 0, size: 32, value: 0x41, fields: [] }
                                ]
                            }
                        ]
                    });
                }
                return Promise.reject('Unknown command');
            }
        } as any as vscode.DebugSession;

        const peripherals = await provider.getChildren(undefined, mockSession);
        assert.strictEqual(peripherals.length, 1);
        assert.strictEqual(peripherals[0].label, 'UART1');

        const registers = await provider.getChildren(peripherals[0], mockSession);
        assert.strictEqual(registers.length, 1);
        assert.strictEqual(registers[0].label, 'DR');
        assert.strictEqual(registers[0].description, '0x00000041');
    });

    test('LabwiredDashboardProvider should post telemetry message', () => {
        const mockWebview = {
            postMessage: (msg: any) => {
                assert.strictEqual(msg.type, 'telemetry');
                assert.strictEqual(msg.data.foo, 'bar');
            }
        } as any;
        const mockView = {
            webview: mockWebview
        } as any as vscode.WebviewView;

        const provider = new LabwiredDashboardProvider(vscode.Uri.file('/fake/path'));
        (provider as any)._view = mockView;
        provider.updateTelemetry({ foo: 'bar' });
    });
});
