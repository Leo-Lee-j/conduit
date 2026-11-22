"""Observe real Codex file-change notifications against a local Responses stub."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import queue
import subprocess
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--codex', required=True)
parser.add_argument('--output', default='target/codex-file-stream-probe')
args = parser.parse_args()
binary = Path(args.codex).resolve(strict=True)
root = Path(args.output).resolve()
root.mkdir(parents=True, exist_ok=True)
home = root / 'home'
home.mkdir(exist_ok=True)
note = root / 'note.txt'
note.write_text('before\n', encoding='utf-8')
requests = []
emitted = []
class Server(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass
    def do_POST(self):
        payload = json.loads(self.rfile.read(int(self.headers.get('Content-Length', '0'))))
        requests.append({'path': self.path, 'tools': payload.get('tools', [])})
        index = len(requests)
        self.send_response(200)
        self.send_header('Content-Type', 'text/event-stream')
        self.send_header('Connection', 'close')
        self.end_headers()
        def emit(kind, **values):
            value = {'type': kind, **values}
            emitted.append({'type': kind, 'time': time.monotonic(), 'file': note.read_text(encoding='utf-8')})
            self.wfile.write(('data: ' + json.dumps(value) + '\n\n').encode())
            self.wfile.flush()
        try:
            emit('response.created', response={'id': f'probe-{index}', 'status': 'in_progress', 'output': []})
            if index == 1:
                item = {'type': 'function_call', 'id': 'fc_patch', 'call_id': 'call_patch', 'name': 'exec_command', 'arguments': ''}
                command = json.dumps({'cmd': '[System.IO.File]::WriteAllText("note.txt", "after`n", [System.Text.UTF8Encoding]::new($false))', 'shell': 'powershell.exe', 'login': False})
                emit('response.output_item.added', output_index=0, item=item)
                for i in range(0, len(command), 7):
                    emit('response.function_call_arguments.delta', item_id='fc_patch', output_index=0, delta=command[i:i+7])
                    time.sleep(.03)
                item['arguments'] = command
                emit('response.function_call_arguments.done', item_id='fc_patch', output_index=0, arguments=command)
                emit('response.output_item.done', output_index=0, item=item)
                output = [item]
            else:
                output = []
            emit('response.completed', response={'id': f'probe-{index}', 'status': 'completed', 'output': output, 'usage': {'input_tokens': 1, 'output_tokens': 1, 'total_tokens': 2}})
        except (BrokenPipeError, ConnectionResetError):
            pass
server = ThreadingHTTPServer(('127.0.0.1', 0), Server)
threading.Thread(target=server.serve_forever, daemon=True).start()
(home / 'config.toml').write_text(f'''model = "gpt-5.4"
model_provider = "probe"
[model_providers.probe]
name = "Local file protocol probe"
base_url = "http://127.0.0.1:{server.server_port}/v1"
wire_api = "responses"
requires_openai_auth = false
[features]
responses_websockets_v2 = false
''', encoding='utf-8')
env = {k: v for k, v in os.environ.items() if not k.startswith(('OPENAI_', 'CODEX_'))}
env['CODEX_HOME'] = str(home)
frames = []
responses = []
q = queue.Queue()
with (root / 'stderr.log').open('w', encoding='utf-8') as errors:
    p = subprocess.Popen([str(binary), 'app-server'], cwd=root, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=errors, text=True, encoding='utf-8')
    def read():
        for line in p.stdout:
            try:
                frame = {'event': json.loads(line), 'time': time.monotonic(), 'file': note.read_text(encoding='utf-8')}
                frames.append(frame)
                q.put(frame['event'])
            except ValueError:
                pass
    reader = threading.Thread(target=read, daemon=True)
    reader.start()
    def rpc(i, method, params):
        p.stdin.write(json.dumps({'id': i, 'method': method, 'params': params}) + '\n')
        p.stdin.flush()
        deadline = time.monotonic() + 25
        while time.monotonic() < deadline:
            value = q.get(timeout=max(.1, deadline - time.monotonic()))
            if value.get('id') == i:
                responses.append(value)
                assert 'error' not in value, value
                return value['result']
        raise TimeoutError(method)
    try:
        rpc(1, 'initialize', {'clientInfo': {'name': 'conduit-file-probe', 'version': '1'}, 'capabilities': {'experimentalApi': True}})
        p.stdin.write('{"method":"initialized"}\n')
        p.stdin.flush()
        thread = rpc(2, 'thread/start', {'cwd': str(root), 'approvalPolicy': 'never', 'sandbox': 'danger-full-access'})['thread']['id']
        rpc(3, 'turn/start', {'threadId': thread, 'input': [{'type': 'text', 'text': 'Apply the local synthetic note.txt patch'}]})
        deadline = time.monotonic() + 25
        while time.monotonic() < deadline:
            value = q.get(timeout=max(.1, deadline - time.monotonic()))
            if value.get('method') == 'turn/completed':
                assert value['params']['turn']['status'] == 'completed', value
                break
        else:
            raise TimeoutError('turn/completed')
    finally:
        p.stdin.close()
        try:
            p.wait(timeout=5)
        except subprocess.TimeoutExpired:
            p.kill()
            p.wait()
        reader.join(timeout=2)
        server.shutdown()
        (root / 'events.json').write_text(json.dumps(frames, indent=2), encoding='utf-8')
        (root / 'model-events.json').write_text(json.dumps(emitted, indent=2), encoding='utf-8')
        (root / 'requests.json').write_text(json.dumps(requests, indent=2), encoding='utf-8')
result = {'version': subprocess.check_output([str(binary), '--version'], text=True).strip(), 'sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'exit_code': p.returncode, 'after': note.read_text(encoding='utf-8'), 'file_events': [f for f in frames if 'fileChange' in f['event'].get('method', '') or f['event'].get('params', {}).get('item', {}).get('type') == 'fileChange']}
result['advertises_apply_patch'] = any(tool.get('name') == 'apply_patch' for request in requests for tool in request['tools'])
result['source'] = 'Real CLI with isolated local Responses SSE; streamed exec_command arguments; default tool set'
(root / 'result.json').write_text(json.dumps(result, indent=2), encoding='utf-8')
print(json.dumps(result, indent=2))
assert result['after'] == 'after\n', 'Native file write did not execute'

assert result['exit_code'] == 0, 'Native CLI did not exit cleanly'
assert any(f['event'].get('method') == 'item/completed' and f['event'].get('params', {}).get('item', {}).get('type') == 'commandExecution' and f['event']['params']['item'].get('exitCode') == 0 for f in frames), 'Native command did not succeed'
