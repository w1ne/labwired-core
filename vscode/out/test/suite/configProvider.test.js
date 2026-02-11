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
const path = require("path");
const extension_1 = require("../../extension");
suite('LabwiredConfigurationProvider Test Suite', () => {
    let mockOutputChannel;
    let mockCpModule;
    let mockCheckFile;
    setup(() => {
        mockOutputChannel = {
            appendLine: (line) => console.log(line)
        };
        mockCpModule = {
            execSync: (cmd) => {
                if (cmd === 'rustc --print sysroot')
                    return '/rust/sysroot';
                if (cmd === 'rustc -vV')
                    return 'commit-hash: abcdef123';
                return '';
            }
        };
        mockCheckFile = (p) => __awaiter(void 0, void 0, void 0, function* () { return true; });
    });
    test('resolveDebugConfiguration should set defaults and auto-detect', () => __awaiter(void 0, void 0, void 0, function* () {
        const provider = new extension_1.LabwiredConfigurationProvider(mockOutputChannel, mockCpModule, mockCheckFile);
        const config = {
            type: '',
            name: '',
            request: ''
        };
        const mockFolder = {
            uri: vscode.Uri.file('/mock/root'),
            name: 'mock',
            index: 0
        };
        const resolved = yield provider.resolveDebugConfiguration(mockFolder, config);
        assert.strictEqual(resolved === null || resolved === void 0 ? void 0 : resolved.type, 'labwired');
        assert.strictEqual(resolved === null || resolved === void 0 ? void 0 : resolved.name, 'LabWired: Launch');
        assert.strictEqual(resolved === null || resolved === void 0 ? void 0 : resolved.request, 'launch');
        assert.strictEqual(resolved === null || resolved === void 0 ? void 0 : resolved.stopOnEntry, true);
        assert.ok(resolved === null || resolved === void 0 ? void 0 : resolved.program.includes('firmware'));
    }));
    test('resolveDebugConfiguration should auto-map Rust source', () => __awaiter(void 0, void 0, void 0, function* () {
        const provider = new extension_1.LabwiredConfigurationProvider(mockOutputChannel, mockCpModule, mockCheckFile);
        const config = {
            type: 'labwired',
            name: 'test',
            request: 'launch',
            program: '/path/to/firmware'
        };
        const resolved = yield provider.resolveDebugConfiguration(undefined, config);
        assert.ok(resolved === null || resolved === void 0 ? void 0 : resolved.sourceMap);
        assert.strictEqual(resolved === null || resolved === void 0 ? void 0 : resolved.sourceMap['/rustc/abcdef123/library'], path.join('/rust/sysroot', 'lib', 'rustlib', 'src', 'rust', 'library'));
    }));
    test('resolveDebugConfiguration should fail if program missing', () => __awaiter(void 0, void 0, void 0, function* () {
        mockCheckFile = (p) => __awaiter(void 0, void 0, void 0, function* () { return false; });
        const provider = new extension_1.LabwiredConfigurationProvider(mockOutputChannel, mockCpModule, mockCheckFile);
        const config = {
            type: 'labwired',
            name: 'test',
            request: 'launch',
            program: '/missing/program'
        };
        const resolved = yield provider.resolveDebugConfiguration(undefined, config);
        assert.strictEqual(resolved, undefined);
    }));
});
//# sourceMappingURL=configProvider.test.js.map
