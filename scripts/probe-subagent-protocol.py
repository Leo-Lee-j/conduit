# No real model endpoint, user session, or authentication store is accessed.
import json, subprocess, threading, queue, os, pathlib, uuid, time
from http.server import ThreadingHTTPServer, BaseHTTPRequestHandler
import argparse
parser = argparse.ArgumentParser(description="Verify Codex child history and targeted controls using a local synthetic model endpoint")
parser.add_argument("--codex", required=True, help="Absolute path to the Codex executable")
parser.add_argument("--output", default="target/subagent-protocol-probe", help="Directory for isolated CLI data and evidence")
args = parser.parse_args()
binary = str(pathlib.Path(args.codex).resolve(strict=True))
root = pathlib.Path(args.output).resolve(); root.mkdir(parents=True, exist_ok=True)
home=root/"home";home.mkdir(exist_ok=True)
release=threading.Event(); requests=[]; request_lock=threading.Lock()
class Server(BaseHTTPRequestHandler):
 def log_message(self,*args):pass
 def do_POST(self):
  body=self.rfile.read(int(self.headers.get("Content-Length",0)))
  with request_lock:
   requests.append({"path":self.path,"bytes":len(body)})
   first=len(requests)==1
  try:
   data=json.loads(body)
   (root/f"request-{len(requests)}.json").write_text(json.dumps(data,indent=2),encoding="utf-8")
  except ValueError:pass
  self.send_response(200);self.send_header("Content-Type","text/event-stream");self.end_headers()
  try:
   self.wfile.write(b'data: {"type":"response.created","response":{"id":"probe-response","object":"response","status":"in_progress","output":[]}}\n\n');self.wfile.flush()
   if first:
    item={"type":"function_call","id":"fc_probe","call_id":"call_probe","name":"spawn_agent","namespace":"multi_agent_v1","arguments":json.dumps({"message":"Wait for synthetic child probe"})}
    for event in [{"type":"response.output_item.added","output_index":0,"item":item},{"type":"response.output_item.done","output_index":0,"item":item},{"type":"response.completed","response":{"id":"probe-response","object":"response","status":"completed","output":[item],"usage":{"input_tokens":1,"output_tokens":1}}}]:
     self.wfile.write(("data: "+json.dumps(event)+"\n\n").encode());self.wfile.flush()
    return
   release.wait(40)
   self.wfile.write(b'data: {"type":"response.completed","response":{"id":"probe-response","object":"response","status":"completed","output":[],"usage":{"input_tokens":1,"output_tokens":0}}}\n\n');self.wfile.flush()
  except (BrokenPipeError,ConnectionResetError):pass
server=ThreadingHTTPServer(("127.0.0.1",0),Server)
threading.Thread(target=server.serve_forever,daemon=True).start()
config=f'''model = "gpt-5.4"
model_provider = "probe"
[model_providers.probe]
name = "Local protocol probe"
base_url = "http://127.0.0.1:{server.server_port}/v1"
wire_api = "responses"
requires_openai_auth = false
[features]
multi_agent = true
responses_websockets_v2 = false
'''
(home/"config.toml").write_text(config,encoding="utf-8")
env=os.environ.copy();env["CODEX_HOME"]=str(home)
q=queue.Queue();events=[];results=[]
p=subprocess.Popen([binary,"app-server"],cwd=root,env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=open(root/"stderr.log","w"),text=True,encoding="utf-8")
def read():
 for line in p.stdout:
  try:q.put(json.loads(line))
  except ValueError:pass
threading.Thread(target=read,daemon=True).start()
def rpc(i,method,params):
 p.stdin.write(json.dumps({"id":i,"method":method,"params":params})+"\n");p.stdin.flush()
 deadline=time.monotonic()+25
 while time.monotonic()<deadline:
  v=q.get(timeout=max(.1,deadline-time.monotonic()))
  if v.get("id")==i:
   results.append({"method":method,"response":v});return v
  events.append(v)
 raise TimeoutError(method)
try:
 rpc(1,"initialize",{"clientInfo":{"name":"conduit-control-probe","version":"1"},"capabilities":{"experimentalApi":True}})
 p.stdin.write(json.dumps({"method":"initialized"})+"\n");p.stdin.flush()
 parent=rpc(2,"thread/start",{"cwd":str(root),"approvalPolicy":"never","sandbox":"read-only"})["result"]["thread"]["id"]
 parent_turn=rpc(4,"turn/start",{"threadId":parent,"input":[{"type":"text","text":"Spawn a synthetic child for isolation probe"}]})["result"]["turn"]["id"]
 deadline=time.monotonic()+25; child=None;child_turn=None
 while time.monotonic()<deadline:
  event=q.get(timeout=max(.1,deadline-time.monotonic()));events.append(event)
  if event.get("method")=="thread/started" and event["params"]["thread"].get("source")!="appServer":
   thread=event["params"]["thread"]
   if thread["id"]!=parent:
    child=thread["id"];results.append({"child_metadata":thread})
  if event.get("method")=="turn/started" and event["params"].get("threadId")!=parent:
   child=event["params"]["threadId"];child_turn=event["params"]["turn"]["id"];break
 assert child and child_turn,"spawn did not publish child turn"
 rpc(3,"thread/read",{"threadId":child,"includeTurns":False})
 bad=rpc(6,"turn/steer",{"threadId":child,"expectedTurnId":"stale","input":[{"type":"text","text":"reject this"}]})
 assert "error" in bad,bad
 good=rpc(7,"turn/steer",{"threadId":child,"expectedTurnId":child_turn,"input":[{"type":"text","text":"synthetic followup"}]})
 assert "result" in good, good
 # This verifies that a stale interrupt cannot stop a newer turn.
 bad_stop=rpc(8,"turn/interrupt",{"threadId":child,"turnId":"stale"})
 good_after=rpc(9,"turn/steer",{"threadId":child,"expectedTurnId":child_turn,"input":[{"type":"text","text":"still active after stale interrupt"}]})
 assert "error" in bad_stop and "result" in good_after, (bad_stop,good_after)
 rpc(10,"turn/interrupt",{"threadId":child,"turnId":child_turn})
 parent_alive=rpc(11,"turn/steer",{"threadId":parent,"expectedTurnId":parent_turn,"input":[{"type":"text","text":"parent isolated"}]})
 assert "result" in parent_alive,parent_alive
 rpc(12,"turn/interrupt",{"threadId":parent,"turnId":parent_turn})
 history=rpc(13,"thread/turns/list",{"threadId":child,"limit":1,"sortDirection":"asc","itemsView":"full"})
 assert "result" in history, history
 resumed=rpc(14,"thread/resume",{"threadId":child,"excludeTurns":True,"cwd":str(root),"approvalPolicy":"never","sandbox":"read-only"})
 assert resumed.get("result",{}).get("thread",{}).get("id")==child,resumed
 continued=rpc(15,"turn/start",{"threadId":child,"input":[{"type":"text","text":"synthetic continuation"}]})
 assert "result" in continued,continued
 rpc(16,"turn/interrupt",{"threadId":child,"turnId":continued["result"]["turn"]["id"]})
 results.append({"checks":{"stale_steer_rejected":True,"correct_steer_accepted":"result" in good,"stale_interrupt_rejected":"error" in bad_stop,"child_survives_stale_interrupt":"result" in good_after,"parent_survives_child_interrupt":True,"local_requests":len(requests)}})
finally:
 release.set();p.stdin.close()
 try:p.wait(timeout=5)
 except subprocess.TimeoutExpired:p.kill();p.wait()
 server.shutdown()
 (root/"results.json").write_text(json.dumps(results,indent=2),encoding="utf-8")
 (root/"events.json").write_text(json.dumps(events,indent=2),encoding="utf-8")
 print(json.dumps(results[-1]))
