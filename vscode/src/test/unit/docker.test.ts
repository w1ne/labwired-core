
import * as assert from 'assert';
import { DockerManager, ILogOutput } from '../../docker';

class MockOutput implements ILogOutput {
    append(value: string): void { }
    appendLine(value: string): void { }
}

suite('DockerManager', () => {
    const mockOutput = new MockOutput();
    const manager = new DockerManager(mockOutput);

    test('should generate correct DAP arguments with mirror mount and port forwarding', () => {
        const image = 'my-image';
        const workspace = '/home/user/project';
        const args = manager.getDapArgs(image, workspace);

        // Check essential components
        assert.ok(args.includes('run'));
        assert.ok(args.includes('-i'));
        assert.ok(args.includes('--rm'));

        // Check Mirror Mount
        const mountIndex = args.indexOf('-v');
        assert.ok(mountIndex !== -1);
        assert.strictEqual(args[mountIndex + 1], '/home/user/project:/home/user/project');

        // Check Port Forwarding
        const portIndex = args.indexOf('-p');
        assert.ok(portIndex !== -1);
        assert.strictEqual(args[portIndex + 1], '9999:9999');

        // Check Workdir
        const workdirIndex = args.indexOf('-w');
        assert.ok(workdirIndex !== -1);
        assert.strictEqual(args[workdirIndex + 1], '/home/user/project');

        // Check Image and Command
        assert.strictEqual(args[args.length - 2], image);
        assert.strictEqual(args[args.length - 1], 'labwired-dap');
    });

    test('should include extra docker arguments', () => {
        const image = 'my-image';
        const workspace = '/ws';
        const extra = ['--privileged', '--network=host'];
        const args = manager.getDapArgs(image, workspace, extra);

        assert.ok(args.includes('--privileged'));
        assert.ok(args.includes('--network=host'));
    });
});
