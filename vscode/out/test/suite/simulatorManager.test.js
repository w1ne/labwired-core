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
const simulatorManager_1 = require("../../simulatorManager");
const events_1 = require("events");
suite('SimulatorManager Test Suite', () => {
    let mockOutputChannel;
    let mockCpModule;
    setup(() => {
        mockOutputChannel = {
            appendLine: (line) => console.log(line),
            append: (data) => console.log(data)
        };
        mockCpModule = {
            spawn: (bin, args) => {
                const process = new events_1.EventEmitter();
                process.stdout = new events_1.EventEmitter();
                process.stderr = new events_1.EventEmitter();
                process.kill = () => process.emit('close', 0);
                return process;
            }
        };
    });
    test('SimulatorManager should start and change status', () => __awaiter(void 0, void 0, void 0, function* () {
        const mgr = new simulatorManager_1.SimulatorManager(mockOutputChannel, mockCpModule);
        assert.strictEqual(mgr.status, simulatorManager_1.SimulatorStatus.Stopped);
        yield mgr.start('test-bin', []);
        assert.strictEqual(mgr.status, simulatorManager_1.SimulatorStatus.Running);
    }));
    test('SimulatorManager should stop and change status', () => __awaiter(void 0, void 0, void 0, function* () {
        const mgr = new simulatorManager_1.SimulatorManager(mockOutputChannel, mockCpModule);
        yield mgr.start('test-bin', []);
        mgr.stop();
        assert.strictEqual(mgr.status, simulatorManager_1.SimulatorStatus.Stopped);
    }));
    test('SimulatorManager should handle process errors', () => __awaiter(void 0, void 0, void 0, function* () {
        const mgr = new simulatorManager_1.SimulatorManager(mockOutputChannel, {
            spawn: () => {
                const process = new events_1.EventEmitter();
                // Emit error after a tick
                setTimeout(() => process.emit('error', new Error('test error')), 0);
                return process;
            }
        });
        yield mgr.start('test-bin', []);
        // Wait for error event
        yield new Promise(resolve => setTimeout(resolve, 10));
        assert.strictEqual(mgr.status, simulatorManager_1.SimulatorStatus.Error);
    }));
});
//# sourceMappingURL=simulatorManager.test.js.map
