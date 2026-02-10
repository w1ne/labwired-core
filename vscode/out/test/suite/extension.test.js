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
// You can import and use all API from the 'vscode' module
// as well as import your extension to test it
const vscode = require("vscode");
// import * as myExtension from '../../extension';
suite('Extension Test Suite', () => {
    vscode.window.showInformationMessage('Start all tests.');
    test('Extension should be present', () => {
        assert.ok(vscode.extensions.getExtension('w1ne.labwired-vscode'));
    });
    test('Configuration defaults', () => {
        const config = vscode.workspace.getConfiguration('labwired');
        const dapPath = config.get('dapPath');
        // By default it should be undefined/empty string if not set, or we can check inspection
        const inspect = config.inspect('dapPath');
        assert.ok(inspect);
    });
    // We can't easily unit test the DescriptorFactory without mocking vscode,
    // but we can verify that the command registered exists.
    test('Commands should be registered', () => __awaiter(void 0, void 0, void 0, function* () {
        const commands = yield vscode.commands.getCommands(true);
        assert.ok(commands.includes('labwired.compileAndRun'));
        assert.ok(commands.includes('labwired.showDashboard'));
    }));
});
//# sourceMappingURL=extension.test.js.map
