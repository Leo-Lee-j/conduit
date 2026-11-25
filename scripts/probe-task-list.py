"""Capture native Claude task tools against an isolated local model stub (no paid API)."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--claude', required=True)
    parser.add_argument('--output', default='target/task-list-probe')
    parser.add_argument('--fixture')
    args = parser.parse_args()
    binary = Path(args.claude).resolve(strict=True)
    root = Path(args.output).resolve()
    root.mkdir(parents=True, exist_ok=True)
    workspace = root / 'workspace'
    workspace.mkdir(exist_ok=True)
    calls = [
        ('TaskCreate', {'subject': 'Inspect the code', 'description': 'Review task tracking.', 'activeForm': 'Inspecting the code'}),
        ('TaskCreate', {'subject': 'Verify the changes', 'description': 'Run focused tests.'}),
        ('TaskUpdate', {'taskId': '1', 'status': 'in_progress', 'owner': 'developer'}),
        ('TaskUpdate', {'taskId': '2', 'addBlockedBy': ['1']}),
        ('TaskList', {}),
        ('TaskGet', {'taskId': '2'}),
        ('TaskUpdate', {'taskId': '1', 'status': 'completed'}),
        ('TaskUpdate', {'taskId': '2', 'subject': 'Verify the task list', 'status': 'in_progress'}),
        ('TaskUpdate', {'taskId': '2', 'status': 'deleted'}),
        ('TaskUpdate', {'taskId': '1', 'status': 'deleted'}),
        ('TaskList', {}),
        ('TaskUpdate', {'taskId': '404', 'status': 'completed'}),
    ]
    requests = []

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass

        def do_POST(self):
            payload = json.loads(self.rfile.read(int(self.headers.get('Content-Length', '0'))))
            if 'count_tokens' in self.path:
                body = b'{"input_tokens":100}'
                self.send_response(200)
                self.end_headers()
                self.wfile.write(body)
                return
            index = len(requests)
            requests.append({'tools': [t for t in payload.get('tools', []) if t.get('name', '').startswith('Task')]})
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.send_header('Connection', 'close')
            self.end_headers()

            def event(kind, **values):
                self.wfile.write(('event: ' + kind + '\ndata: ' + json.dumps({'type': kind, **values}) + '\n\n').encode())
                self.wfile.flush()

            event('message_start', message={'id': f'msg_task_{index}', 'type': 'message', 'role': 'assistant', 'model': payload.get('model', 'probe'), 'content': [], 'stop_reason': None, 'stop_sequence': None, 'usage': {'input_tokens': 100, 'output_tokens': 1}})
            if index < len(calls):
                name, data = calls[index]
                event('content_block_start', index=0, content_block={'type': 'tool_use', 'id': f'tool_task_{index}', 'name': name, 'input': {}})
                event('content_block_delta', index=0, delta={'type': 'input_json_delta', 'partial_json': json.dumps(data)})
                reason = 'tool_use'
            else:
                event('content_block_start', index=0, content_block={'type': 'text', 'text': ''})
                event('content_block_delta', index=0, delta={'type': 'text_delta', 'text': 'Local task-list probe complete.'})
                reason = 'end_turn'
            event('content_block_stop', index=0)
            event('message_delta', delta={'stop_reason': reason, 'stop_sequence': None}, usage={'output_tokens': 10})
            event('message_stop')

    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    env = os.environ.copy()
    for key in list(env):
        if key.startswith(('ANTHROPIC_', 'CLAUDE_', 'CLAUDECODE')):
            del env[key]
    env.update(ANTHROPIC_BASE_URL=f'http://127.0.0.1:{server.server_port}', ANTHROPIC_API_KEY='local-probe-only', CLAUDE_CONFIG_DIR=str(root / 'home'), CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC='1', CLAUDE_CODE_ENABLE_TASKS='true')
    names = 'TaskCreate,TaskUpdate,TaskList,TaskGet'
    command = [str(binary), '--print', '--input-format', 'stream-json', '--output-format', 'stream-json', '--include-partial-messages', '--verbose', '--tools', names, '--allowedTools', names, '--settings', '{"disableAllHooks":true}', '--strict-mcp-config', '--mcp-config', '{"mcpServers":{}}', '--max-turns', str(len(calls) + 2), '--model', 'claude-sonnet-4-6']
    with (root / 'stderr.log').open('w', encoding='utf-8') as errors:
        process = subprocess.Popen(command, cwd=workspace, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=errors, text=True, encoding='utf-8')
        try:
            stdout, _ = process.communicate(json.dumps({'type': 'user', 'message': {'role': 'user', 'content': 'Exercise the local task tools.'}}) + '\n', timeout=55)
        finally:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=5)
            server.shutdown()
    frames = [json.loads(line) for line in stdout.splitlines() if line.strip()]
    (root / 'events.json').write_text(json.dumps(frames, indent=2), encoding='utf-8')
    (root / 'schemas.json').write_text(json.dumps(requests[:1], indent=2), encoding='utf-8')
    print(json.dumps({'exit_code': process.returncode, 'requests': len(requests), 'events': len(frames)}))
    assert process.returncode == 0
    assert len(requests) == len(calls) + 1
    clean = []
    for value in frames:
        if value['type'] == 'assistant':
            clean.append({'type': 'assistant', 'message': {key: value['message'][key] for key in ('id', 'content')}})
        elif value['type'] == 'user':
            clean.append({key: value[key] for key in ('type', 'message', 'tool_use_result') if key in value})
    assert any(isinstance(frame.get('tool_use_result'), dict) and frame['tool_use_result'].get('task') for frame in clean), 'Native task tools unavailable'
    results = [frame for frame in clean if frame['type'] == 'user']
    assert len(results) == len(calls), 'Task tool result sequence missing'
    assert results[0]['tool_use_result']['task']['id'] == '1' and results[1]['tool_use_result']['task']['id'] == '2', 'Use a fresh output directory'
    for frame in results[:-1]:
        assert isinstance(frame.get('tool_use_result'), dict), 'Structured task result missing'
        assert frame['tool_use_result'].get('success') is not False, 'Unexpected native task failure'
        assert all(not block.get('is_error') for block in frame['message']['content']), 'Unexpected tool error'
    assert results[-2]['tool_use_result']['tasks'] == [], 'Explicit empty snapshot missing'
    assert results[-1]['tool_use_result']['success'] is False, 'Expected missing-task failure missing'
    if args.fixture:
        native = []
        for path in (root / 'home' / 'projects').rglob('*.jsonl'):
            for line in path.read_text(encoding='utf-8').splitlines():
                value = json.loads(line)
                if value.get('type') not in ('user', 'assistant'):
                    continue
                frame = {key: value[key] for key in ('type', 'toolUseResult', 'promptSource', 'isSidechain') if key in value}
                frame['message'] = {key: value['message'][key] for key in ('id', 'role', 'content') if key in value['message']}
                frame.update(cwd='C:/fixture', sessionId='task-fixture')
                native.append(frame)
        assert any('toolUseResult' in frame for frame in native), 'Native JSONL task results missing'
        fixture = Path(args.fixture)
        fixture.parent.mkdir(parents=True, exist_ok=True)
        fixture.write_text(json.dumps({'cli': subprocess.check_output([str(binary), '--version'], text=True).strip(), 'source': 'Real CLI; isolated local synthetic Anthropic SSE; native stdout and JSONL; synthetic task content; paths and session IDs normalized', 'frames': clean, 'native_frames': native}, indent=2) + '\n', encoding='utf-8')


if __name__ == '__main__':
    main()
