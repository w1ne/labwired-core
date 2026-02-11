"use strict";
var __awaiter = (this && this.__awaiter) || function (thisArg, _arguments, P, generator) {
    function adopt(value) { return value instanceof P ? value : new P(function (resolve) { resolve(value); }); }
    return new (P || (P = Promise))(function (resolve, reject) {
        function fulfilled(value) { try { step(generator.next(value)); } catch (e) { reject(e); } }
        function rejected(value) { try { step(generator["throw"](value)); } catch (e) { reject(e); } }
        function step(result) { result.done ? resolve(result.value) : adopt(result.value).then(fulfilled, rejected); }
        step((generator = generator.apply(thisArg, _arguments || [])).next());
    });
};
Object.defineProperty(exports, "__esModule", { value: true });
const assert = require("assert");
const vscode = require("vscode");
const dashboard_1 = require("../../dashboard");
const peripheralProvider_1 = require("../../peripheralProvider");
const rtosProvider_1 = require("../../rtosProvider");
suite('Providers Test Suite', () => {
    vscode.window.showInformationMessage('Start all tests.');
    test('PeripheralProvider should handle missing debug session', () => __awaiter(void 0, void 0, void 0, function* () {
        const provider = new peripheralProvider_1.PeripheralProvider();
        const children = yield provider.getChildren();
        assert.strictEqual(children.length, 0);
    }));
    test('PeripheralProvider refresh should fire event', (done) => {
        const provider = new peripheralProvider_1.PeripheralProvider();
        provider.onDidChangeTreeData(() => {
            done();
        });
        provider.refresh();
    });
    test('RTOSProvider should handle missing debug session', () => __awaiter(void 0, void 0, void 0, function* () {
        const provider = new rtosProvider_1.RTOSProvider();
        const children = yield provider.getChildren();
        assert.strictEqual(children.length, 0);
    }));
    test('RTOSProvider should parse tasks from customRequest', () => __awaiter(void 0, void 0, void 0, function* () {
        const provider = new rtosProvider_1.RTOSProvider();
        const mockSession = {
            customRequest: (command) => {
                if (command === 'readRTOSState') {
                    return Promise.resolve({
                        tasks: [
                            { name: 'TestTask', state: 'Running', stackUsage: 50, priority: 1 }
                        ]
                    });
                }
                return Promise.reject('Unknown command');
            }
        };
        const children = yield provider.getChildren(undefined, mockSession);
        assert.strictEqual(children.length, 1);
        assert.strictEqual(children[0].label, 'TestTask');
        assert.strictEqual(children[0].description, 'Running');
    }));
    test('PeripheralProvider should parse peripherals and registers', () => __awaiter(void 0, void 0, void 0, function* () {
        const provider = new peripheralProvider_1.PeripheralProvider();
        const mockSession = {
            customRequest: (command) => {
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
        };
        const peripherals = yield provider.getChildren(undefined, mockSession);
        assert.strictEqual(peripherals.length, 1);
        assert.strictEqual(peripherals[0].label, 'UART1');
        const registers = yield provider.getChildren(peripherals[0], mockSession);
        assert.strictEqual(registers.length, 1);
        assert.strictEqual(registers[0].label, 'DR');
        assert.strictEqual(registers[0].description, '0x00000041');
    }));
    test('LabwiredDashboardProvider should post telemetry message', () => {
        const mockWebview = {
            postMessage: (msg) => {
                assert.strictEqual(msg.type, 'telemetry');
                assert.strictEqual(msg.data.foo, 'bar');
            }
        };
        const mockView = {
            webview: mockWebview
        };
        const provider = new dashboard_1.LabwiredDashboardProvider(vscode.Uri.file('/fake/path'));
        provider._view = mockView;
        provider.updateTelemetry({ foo: 'bar' });
    });
});
//# sourceMappingURL=providers.test.js.map
