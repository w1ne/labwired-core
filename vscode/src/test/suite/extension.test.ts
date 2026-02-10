import * as assert from 'assert';

// You can import and use all API from the 'vscode' module
// as well as import your extension to test it
import * as vscode from 'vscode';
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
    test('Commands should be registered', async () => {
        const commands = await vscode.commands.getCommands(true);
        assert.ok(commands.includes('labwired.compileAndRun'));
        assert.ok(commands.includes('labwired.showDashboard'));
    });
});
