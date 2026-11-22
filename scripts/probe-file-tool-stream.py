"""Capture file-tool events from a real CLI against an isolated local model stub."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--claude', required=True, help='Absolute native Claude executable')
    parser.add_argument('--output', default='target/file-tool-stream-probe')
    parser.add_argument('--fixture', help='Optional sanitized protocol fixture path')
    args = parser.parse_args()
    binary = Path(args.claude).resolve(strict=True)
    root = Path(args.output).resolve()
    root.mkdir(parents=True, exist_ok=True)
    workspace = root / 'workspace'
    workspace.mkdir(exist_ok=True)
    note = workspace / 'note.txt'
    before = 'before\n'
    written = 'after\n\u4e2d\u6587 e\u0301 \U0001f468\u200d\U0001f469\u200d\U0001f467\u200d\U0001f466\n'
    after = written.replace('after', 'updated')
    note.write_text(before, encoding='utf-8', newline='')
    requests = []
    lock = threading.Lock()

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass

        def do_POST(self):
            payload = json.loads(self.rfile.read(int(self.headers.get('Content-Length', '0'))))
            if 'count_tokens' in self.path:
                self.send_response(200)
                self.send_header('Content-Type', 'application/json')
                self.end_headers()
                self.wfile.write(b'{"input_tokens":100}')
                return
            with lock:
                index = len(requests)
                requests.append({'path': self.path, 'stream': payload.get('stream')})
            tool = [
                ('Read', {'file_path': 'note.txt'}),
                ('Edit', {'file_path': 'note.txt', 'old_string': before, 'new_string': written}),
                ('Edit', {'file_path': 'note.txt', 'old_string': 'after', 'new_string': 'updated'}),
            ]
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.send_header('Connection', 'close')
            self.end_headers()
            def event(kind, **values):
                value = {'type': kind, **values}
                self.wfile.write(('event: ' + kind + '\ndata: ' + json.dumps(value) + '\n\n').encode())
                self.wfile.flush()
            try:
                event('message_start', message={'id': f'msg_probe_{index}', 'type': 'message', 'role': 'assistant', 'model': payload.get('model', 'probe'), 'content': [], 'stop_reason': None, 'stop_sequence': None, 'usage': {'input_tokens': 100, 'output_tokens': 1}})
                if index < len(tool):
                    name, data = tool[index]
                    event('content_block_start', index=0, content_block={'type': 'tool_use', 'id': f'tool_probe_{index}', 'name': name, 'input': {}})
                    wire = json.dumps(data, ensure_ascii=True)
                    for start in range(0, len(wire), 7):
                        event('content_block_delta', index=0, delta={'type': 'input_json_delta', 'partial_json': wire[start:start+7]})
                        time.sleep(.015)
                    reason = 'tool_use'
                else:
                    event('content_block_start', index=0, content_block={'type': 'text', 'text': ''})
                    event('content_block_delta', index=0, delta={'type': 'text_delta', 'text': 'Local file-tool probe complete.'})
                    reason = 'end_turn'
                event('content_block_stop', index=0)
                event('message_delta', delta={'stop_reason': reason, 'stop_sequence': None}, usage={'output_tokens': 10})
                event('message_stop')
            except (BrokenPipeError, ConnectionResetError):
                pass

    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    env = os.environ.copy()
    for key in list(env):
        if key.startswith(('ANTHROPIC_', 'CLAUDE_', 'CLAUDECODE')):
            del env[key]
    env.update(ANTHROPIC_BASE_URL=f'http://127.0.0.1:{server.server_port}', ANTHROPIC_API_KEY='local-probe-only', CLAUDE_CONFIG_DIR=str(root / 'home'), CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC='1')
    command = [str(binary), '--bare', '--print', '--input-format', 'stream-json', '--output-format', 'stream-json', '--include-partial-messages', '--verbose', '--no-session-persistence', '--tools', 'Read,Write,Edit', '--allowedTools', 'Read,Write,Edit', '--permission-mode', 'acceptEdits', '--max-turns', '5', '--model', 'claude-sonnet-4-6']
    frames = []
    with (root / 'stderr.log').open('w', encoding='utf-8') as errors:
        process = subprocess.Popen(command, cwd=workspace, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=errors, text=True, encoding='utf-8')
        process.stdin.write(json.dumps({'type': 'user', 'message': {'role': 'user', 'content': 'Exercise the local Read and Edit tools on note.txt.'}}) + '\n')
        process.stdin.close()
        timer = threading.Timer(50, process.kill)
        timer.start()
        try:
            for line in process.stdout:
                value = json.loads(line)
                frames.append({'event': value, 'file': note.read_text(encoding='utf-8')})
            code = process.wait(timeout=5)
        finally:
            timer.cancel()
            if process.poll() is None:
                process.kill()
                process.wait(timeout=5)
            server.shutdown()
    (root / 'events.json').write_text(json.dumps(frames, indent=2, ensure_ascii=False), encoding='utf-8')
    result = {'exit_code': code, 'requests': requests, 'events': len(frames), 'before': before, 'after': note.read_text(encoding='utf-8'), 'expected': after, 'sha256': hashlib.sha256(binary.read_bytes()).hexdigest()}
    (root / 'result.json').write_text(json.dumps(result, indent=2, ensure_ascii=False), encoding='utf-8')
    print(json.dumps({k: v for k, v in result.items() if k not in ('before', 'after', 'expected')}, indent=2))
    assert code == 0, f'CLI failed; inspect {root / "stderr.log"}'
    assert len(requests) == 4, 'Read/Edit/Edit/final request sequence missing'
    assert note.read_text(encoding='utf-8') == after, 'Real file-tool results did not match'
    assert any(frame['event'].get('event', {}).get('delta', {}).get('type') == 'input_json_delta' for frame in frames), 'Native parameter deltas missing'
    assert all(not block.get('is_error') for frame in frames for block in frame['event'].get('message', {}).get('content', []) if isinstance(block, dict) and block.get('type') == 'tool_result'), 'Native tool failure'
    assert any(frame['event'].get('type') == 'result' and frame['event'].get('subtype') == 'success' and not frame['event'].get('is_error') for frame in frames), 'Native success missing'
    if args.fixture:
        clean = []
        for frame in frames:
            value = frame['event']
            kind = value['type']
            if kind == 'stream_event':
                event = {'type': kind, 'event': value['event']}
                if event['event']['type'] == 'message_start':
                    event['event'] = {'type': 'message_start', 'message': {'id': value['event']['message']['id']}}
            elif kind == 'assistant':
                event = {'type': kind, 'message': {key: value['message'][key] for key in ('id', 'content')}}
            elif kind == 'user':
                event = {'type': kind, 'message': {'content': [{key: block[key] for key in ('type', 'tool_use_id', 'is_error') if key in block} for block in value['message']['content']]}}
            elif kind == 'result':
                event = {key: value[key] for key in ('type', 'subtype', 'is_error')}
            else:
                continue
            clean.append({'event': event, 'file': frame['file']})
        fixture = Path(args.fixture)
        fixture.parent.mkdir(parents=True, exist_ok=True)
        fixture.write_text(json.dumps({'cli': subprocess.check_output([str(binary), '--version'], text=True).strip(), 'sha256': result['sha256'], 'source': 'Real CLI; isolated local synthetic Anthropic SSE; relative file paths; ordered native stdout; filesystem snapshot per frame', 'frames': clean}, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')


if __name__ == '__main__':
    main()