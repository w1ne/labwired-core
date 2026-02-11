import * as assert from 'assert';
import * as vscode from 'vscode';
import * as path from 'path';
import { LabwiredConfigurationProvider } from '../../extension';

suite('LabwiredConfigurationProvider Test Suite', () => {
    let mockOutputChannel: any;
    let mockCpModule: any;
    let mockCheckFile: any;

    setup(() => {
        mockOutputChannel = {
            appendLine: (line: string) => console.log(line)
        };
        mockCpModule = {
            execSync: (cmd: string) => {
                if (cmd === 'rustc --print sysroot') return '/rust/sysroot';
                if (cmd === 'rustc -vV') return 'commit-hash: abcdef123';
                return '';
            }
        };
        mockCheckFile = async (p: string) => true;
    });

    test('resolveDebugConfiguration should set defaults and auto-detect', async () => {
        const provider = new LabwiredConfigurationProvider(mockOutputChannel, mockCpModule, mockCheckFile);
        const config: vscode.DebugConfiguration = {
            type: '',
            name: '',
            request: ''
        };
        const mockFolder = {
            uri: vscode.Uri.file('/mock/root'),
            name: 'mock',
            index: 0
        } as vscode.WorkspaceFolder;

        const resolved = await provider.resolveDebugConfiguration(mockFolder, config);
        assert.strictEqual(resolved?.type, 'labwired');
        assert.strictEqual(resolved?.name, 'LabWired: Launch');
        assert.strictEqual(resolved?.request, 'launch');
        assert.strictEqual(resolved?.stopOnEntry, true);
        assert.ok(resolved?.program.includes('firmware'));
    });

    test('resolveDebugConfiguration should auto-map Rust source', async () => {
        const provider = new LabwiredConfigurationProvider(mockOutputChannel, mockCpModule, mockCheckFile);
        const config: vscode.DebugConfiguration = {
            type: 'labwired',
            name: 'test',
            request: 'launch',
            program: '/path/to/firmware'
        };

        const resolved = await provider.resolveDebugConfiguration(undefined, config);
        assert.ok(resolved?.sourceMap);
        assert.strictEqual(resolved?.sourceMap['/rustc/abcdef123/library'], path.join('/rust/sysroot', 'lib', 'rustlib', 'src', 'rust', 'library'));
    });

    test('resolveDebugConfiguration should fail if program missing', async () => {
        mockCheckFile = async (p: string) => false;
        const provider = new LabwiredConfigurationProvider(mockOutputChannel, mockCpModule, mockCheckFile);
        const config: vscode.DebugConfiguration = {
            type: 'labwired',
            name: 'test',
            request: 'launch',
            program: '/missing/program'
        };

        const resolved = await provider.resolveDebugConfiguration(undefined, config);
        assert.strictEqual(resolved, undefined);
    });
});
