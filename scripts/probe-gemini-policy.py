#!/usr/bin/env python3
"""Windows Gemini CLI ACP smoke test using a local fake model, without paid calls.

Usage: python scripts/probe-gemini-policy.py --cli <gemini-cli/bundle/gemini.js>
Requires Node.js and an installed @google/gemini-cli 0.71.1-compatible package.
Uses a temporary native home/workspace; never changes the real user's login.
Existing enterprise system policies are intentionally not bypassed.
"""
import argparse
import tempfile
import os, json, subprocess, pathlib, shutil, queue, threading, time, http.server

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--cli', required=True, type=pathlib.Path)
cli = parser.parse_args().cli.resolve(strict=True)
fixture = tempfile.TemporaryDirectory(prefix='conduit-gemini-policy-')
root = pathlib.Path(fixture.name)
workspace = root / 'policy-workspace'
workspace.mkdir(exist_ok=True)
(workspace / 'read.txt').write_text('CONDUIT_READ_OK', encoding='utf-8')
probe_home = root / 'policy-home'
(probe_home / '.gemini').mkdir(parents=True, exist_ok=True)
(probe_home / '.gemini/settings.json').write_text(json.dumps({'security': {'auth': {'selectedType': 'gemini-api-key'}}}))
reads = ['list_directory', 'read_file', 'grep_search', 'glob']

def verify(mode, resume=None):
    requests, permissions, updates, errors = [], [], [], []
    attempts = ['read_file', 'run_shell_command'] if mode != 'text' else ['run_shell_command']
    class Model(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args): pass
        def do_POST(self):
            req = json.loads(self.rfile.read(int(self.headers.get('Content-Length', '0'))) or b'{}')
            streaming = 'streamGenerateContent' in self.path
            if streaming:
                requests.append(req)
                index = len(requests) - 1
                if index < len(attempts):
                    tool = attempts[index]
                    args = {'file_path': str(workspace / 'read.txt')} if tool == 'read_file' else {'command': 'echo BLOCKED > "' + str(workspace / ('unexpected-' + mode + '.txt')) + '"'}
                    parts = [{'functionCall': {'name': tool, 'args': args}}]
                else: parts = [{'text': 'CONDUIT_GEMINI_OK'}]
            else: parts = [{'text': 'CONDUIT_GEMINI_OK'}]
            value = {'candidates': [{'content': {'role': 'model', 'parts': parts}, 'finishReason': 'STOP', 'index': 0}], 'usageMetadata': {'promptTokenCount': 10, 'candidatesTokenCount': 5, 'totalTokenCount': 15}}
            body = ('data: ' + json.dumps(value) + '\n\n').encode() if streaming else json.dumps(value).encode()
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream' if streaming else 'application/json')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Model)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    rules = ['[[rule]]\ntoolName="*"\ndecision="' + ('ask_user' if mode == 'workspace' else 'deny') + '"\npriority=998\n']
    if mode == 'readonly':
        rules += ['[[rule]]\ntoolName="' + name + '"\ndecision="allow"\npriority=999\n' for name in reads]
    policy = root / ('native-' + mode + '.toml')
    policy.write_text('\n'.join(rules))
    env = dict(os.environ, GEMINI_API_KEY='conduit-offline-probe', GOOGLE_GEMINI_BASE_URL='http://127.0.0.1:' + str(server.server_port), HOME=str(probe_home), USERPROFILE=str(probe_home), GEMINI_CLI_HOME=str(probe_home))
    # No synthetic system override: the actual native policy must enforce the boundary.
    # Keep genuine enterprise system settings/policies in force.
    args = [shutil.which('node'), str(cli), '--acp', '--approval-mode', 'default', '--admin-policy', str(policy), '--extensions', 'none']
    if mode != 'workspace': args += ['--allowed-mcp-server-names', '__conduit_no_mcp_servers__']
    process = subprocess.Popen(args, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, encoding='utf-8', errors='replace', env=env, cwd=workspace, creationflags=subprocess.CREATE_NO_WINDOW)
    incoming = queue.Queue()
    def reader(stream, target):
        for line in stream: target.put(line)
    threading.Thread(target=reader, args=(process.stdout, incoming), daemon=True).start()
    threading.Thread(target=reader, args=(process.stderr, queue.Queue()), daemon=True).start()
    def write(value):
        process.stdin.write(json.dumps(value) + '\n'); process.stdin.flush()
    def rpc(id, method, params):
        write({'jsonrpc': '2.0', 'id': id, 'method': method, 'params': params})
        deadline = time.monotonic() + 40
        while time.monotonic() < deadline:
            try: line = incoming.get(timeout=1)
            except queue.Empty: continue
            try: value = json.loads(line)
            except ValueError: continue
            if value.get('id') == id: return value
            if value.get('method') == 'session/update': updates.append(value['params']['update'])
            if value.get('method') == 'session/request_permission':
                permissions.append(value)
                option = next((o['optionId'] for o in value['params']['options'] if o['kind'] == 'reject_once'), None)
                outcome = {'outcome': 'selected', 'optionId': option} if option else {'outcome': 'cancelled'}
                write({'jsonrpc': '2.0', 'id': value['id'], 'result': {'outcome': outcome}})
        raise TimeoutError(method)
    try:
        assert 'result' in rpc(1, 'initialize', {'protocolVersion': 1, 'clientCapabilities': {'fs': {'readTextFile': False, 'writeTextFile': False}, 'terminal': False}})
        params = {'cwd': str(workspace), 'mcpServers': []}
        if resume: params['sessionId'] = resume
        session = rpc(2, 'session/load' if resume else 'session/new', params)
        assert 'result' in session, session
        sid = resume or session['result']['sessionId']
        assert 'result' in rpc(3, 'session/set_mode', {'sessionId': sid, 'modeId': 'default'})
        result = rpc(4, 'session/prompt', {'sessionId': sid, 'prompt': [{'type': 'text', 'text': 'Run the requested fixture tools then reply CONDUIT_GEMINI_OK.'}]})
        payload = json.dumps(requests)
        summary = {'mode': mode, 'restored': bool(resume), 'stopReason': result.get('result', {}).get('stopReason'), 'approvals': len(permissions), 'read_content_reached_model': 'CONDUIT_READ_OK' in payload, 'write_occurred': (workspace / ('unexpected-' + mode + '.txt')).exists(), 'tool_statuses': [(u.get('title'), u.get('status')) for u in updates if u.get('sessionUpdate') in ['tool_call', 'tool_call_update']]}
        print(json.dumps(summary), flush=True)
        assert summary['stopReason'] == 'end_turn'
        assert not summary['write_occurred']
        if mode == 'readonly': assert summary['read_content_reached_model'] and not permissions
        if mode == 'text': assert not permissions
        if mode == 'workspace': assert permissions
        return sid
    finally:
        if process.poll() is None: subprocess.run(['taskkill', '/PID', str(process.pid), '/T', '/F'], capture_output=True, creationflags=subprocess.CREATE_NO_WINDOW)
        process.wait(timeout=5)
        server.shutdown()

sid = verify('readonly')
verify('text', sid)
verify('workspace')
fixture.cleanup()
