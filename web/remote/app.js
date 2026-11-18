"use strict";

const $ = (id) => document.getElementById(id);
const apiPath = "/api/remote/v1";
const stateLabels = {
  idle: "空闲", busy: "原生会话正在其他标签运行", pending_save: "正在保存消息",
  running: "运行中", waiting_input: "请在电脑回答", waiting_approval: "请在电脑审批",
  cancelling: "正在停止", cleanup_blocked: "电脑正在重新清理进程",
};
const model = {
  auth: null, ready: false, system: null, workspace: null, session: null, archived: false,
  sessions: [], history: [], latest: [], earlierCursor: null, nextSession: null,
  eventSource: null, timer: null, updating: false, again: false, online: false,
  pending: null, checking: false, metadata: null, generation: 0,
};
let pairSecret = new URLSearchParams(location.hash.slice(1)).get("pair");
if (location.hash) history.replaceState(null, "", location.pathname);

function stored(key, value) {
  try {
    if (value === undefined) return sessionStorage.getItem(key);
    if (value === null) sessionStorage.removeItem(key);
    else sessionStorage.setItem(key, value);
  } catch { /* A private browser can deny tab storage. */ }
}

function notice(text = "") {
  $("notice").textContent = text;
  $("notice").hidden = !text;
}

class HttpError extends Error {
  constructor(status, message) { super(message); this.status = status; }
}

async function api(path, options = {}) {
  const response = await fetch(apiPath + path, {
    cache: "no-store", credentials: "same-origin", ...options,
    signal: AbortSignal.timeout(10000),
  });
  if (response.status === 204) return null;
  const data = await response.json().catch(() => null);
  if (!response.ok) throw new HttpError(response.status, data?.message || `请求失败 (${response.status})`);
  return data;
}

function online(value) {
  model.online = value;
  $("connection").dataset.online = String(value);
  $("connection").textContent = value ? "电脑已连接" : "电脑未连接";
  controls();
}

function connectivity(error) {
  online(false);
  if (error.status === 401) {
    model.generation++;
    model.auth = null;
    model.ready = false;
    model.eventSource?.close();
    model.eventSource = null;
    show();
    if (model.pending) notice("配对已失效。待确认的请求不会自动重发，请配对后先检查会话。");
  }
}

function controls() {
  const session = model.session;
  const blocked = !model.online || !model.ready || !model.system?.writable;
  $("new-session").disabled = blocked || !!model.pending;
  $("manage").disabled = blocked || !session || !!model.pending;
  $("send").disabled = blocked || !!model.pending || !session || session.archived || session.state !== "idle" || !model.chatReady || session.queued || !$("draft").value.trim();
  $("stop").hidden = !session?.run_id || ["cancelling", "cleanup_blocked"].includes(session.state);
  $("stop").disabled = blocked;
  $("hint").textContent = model.pending?.kind === "send" ? "正在确认保存结果…" : session?.archived ? "取消归档后可发送" : session?.queued ? "电脑上有待发消息" : "";
}

function show() {
  $("pairing").hidden = !!model.auth;
  $("pair-form").hidden = !pairSecret;
  $("home").hidden = !model.auth || !!model.session;
  $("chat").hidden = !model.auth || !model.session;
  controls();
}

function events() {
  if (model.eventSource) return;
  const stream = new EventSource(apiPath + "/events");
  model.eventSource = stream;
  stream.addEventListener("refresh", refreshSoon);
  stream.onerror = () => refreshSoon();
}

function refreshSoon() {
  if (document.hidden || model.timer) return;
  model.timer = setTimeout(() => { model.timer = null; void refresh(); }, 180);
}

async function refresh() {
  if (document.hidden || !model.auth) return;
  if (model.updating) { model.again = true; return; }
  model.updating = true;
  const generation = model.generation;
  try {
    if (!model.ready) {
      const [system, workspaces] = await Promise.all([api("/system"), api("/workspaces")]);
      if (generation !== model.generation) return;
      model.system = { ...system, local_time: Math.floor(Date.now() / 1000) };
      model.workspace = workspaces.items[0];
      model.ready = true;
      $("workspace").textContent = model.workspace?.name || "工作区";
      events();
    }
    if (model.session) {
      const id = model.session.id;
      const [session, messages] = await Promise.all([api(`/sessions/${id}`), api(`/sessions/${id}/messages`)]);
      if (generation !== model.generation) return;
      model.session = session;
      model.chatReady = messages.transcript_state === "ready";
      model.latest = messages.items;
      if (!model.history.length) model.earlierCursor = messages.next_cursor;
      $("title").textContent = session.title;
      $("state").textContent = session.engine + " · " + stateLabels[session.state];
      renderMessages(messages.transcript_state);
    } else if (model.workspace) {
      const page = await api(sessionPath());
      if (generation !== model.generation) return;
      // Live refresh replaces only the first page and leaves loaded older rows usable.
      if (model.sessions.length <= 30) {
        model.sessions = page.items;
        model.nextSession = page.next_cursor;
      } else {
        const seen = new Set(page.items.map(({id}) => id));
        model.sessions = page.items.concat(model.sessions.slice(30).filter(({id}) => !seen.has(id)));
      }
      renderSessions();
    }
    online(true);
    void checkPending();
  } catch (error) {
    if (generation !== model.generation) return;
    if (error.status === 404 && model.session) {
      exitChat();
      notice("此会话已从工作区移除。");
    } else connectivity(error);
  } finally {
    model.updating = false;
    if (model.again) { model.again = false; refreshSoon(); }
  }
}

function sessionPath(cursor) {
  const params = new URLSearchParams({ archived: String(model.archived) });
  if (cursor) params.set("cursor", cursor);
  return `/workspaces/${model.workspace.id}/sessions?${params}`;
}

function renderSessions() {
  const items = model.sessions.map((session) => {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "session";
    const info = document.createElement("span");
    const title = document.createElement("strong");
    title.textContent = session.title;
    const summary = document.createElement("span");
    summary.className = "summary";
    summary.textContent = [session.engine, session.pinned && "置顶", stateLabels[session.state]].filter(Boolean).join(" · ");
    const arrow = document.createElement("span");
    arrow.className = "arrow";
    arrow.setAttribute("aria-hidden", "true");
    arrow.textContent = "›";
    info.append(title, summary);
    button.append(info, arrow);
    button.addEventListener("click", () => openChat(session));
    return button;
  });
  if (!items.length) {
    const empty = document.createElement("p");
    empty.className = "muted";
    empty.textContent = model.archived ? "暂无已归档会话。" : "工作区暂无会话。";
    items.push(empty);
  }
  $("sessions").replaceChildren(...items);
  $("more-sessions").hidden = !model.nextSession;
}

function renderMessages(transcript, forceBottom = false) {
  const scroll = $("messages-scroll");
  const bottom = forceBottom || scroll.scrollHeight - scroll.scrollTop - scroll.clientHeight < 70;
  const combined = new Map([...model.history, ...model.latest].map((item) => [item.id, item]));
  const items = [...combined.values()].map((item) => {
    const article = document.createElement("article");
    article.className = "message";
    article.dataset.kind = item.kind;
    const role = document.createElement("h2");
    role.textContent = item.kind === "user" ? "你" : item.kind === "error" ? "运行错误" : "助手";
    const content = document.createElement("pre");
    content.textContent = item.kind === "error" ? "本轮运行遇到错误，请在电脑查看详情。" : item.text_preview + (item.content_truncated ? "\n… 请在电脑查看完整内容" : "");
    article.append(role, content);
    return article;
  });
  if (!items.length) {
    const empty = document.createElement("p");
    empty.className = "muted";
    empty.textContent = transcript === "failed" ? "电脑未能载入历史。" : transcript === "ready" ? "暂无消息。" : "正在载入历史…";
    items.push(empty);
  }
  $("messages").replaceChildren(...items);
  $("older").hidden = !model.earlierCursor;
  if (bottom) scroll.scrollTop = scroll.scrollHeight;
}

function draftKey(id) { return `conduit-draft-${id}`; }

function openChat(session) {
  model.generation++;
  model.session = session;
  model.chatReady = false;
  model.history = [];
  model.latest = [];
  model.earlierCursor = null;
  $("draft").value = stored(draftKey(session.id)) || "";
  $("title").textContent = session.title;
  $("state").textContent = stateLabels[session.state] || "";
  show();
  renderMessages("loading", true);
  refreshSoon();
}

function exitChat() {
  model.generation++;
  model.session = null;
  model.sessions = [];
  model.nextSession = null;
  $("sessions").replaceChildren();
  show();
  refreshSoon();
}

function remember(pending) {
  model.pending = pending;
  stored("conduit-pending", pending ? JSON.stringify(pending) : null);
  controls();
}

function requestInfo(path, kind, body, version, method = "POST") {
  return {
    id: crypto.randomUUID(), epoch: model.system.host_epoch, device: model.auth.device_id,
    expires: Math.floor(Date.now() / 1000) + model.system.server_time - model.system.local_time + 540,
    path, kind, body: body === undefined ? undefined : JSON.stringify(body), version, method,
    session: model.session?.id,
  };
}

function submit(request) {
  const headers = {
    "X-CSRF-Token": model.auth.csrf_token, "X-Conduit-Epoch": request.epoch,
    "Idempotency-Key": request.id, "X-Conduit-Request-Expires": String(request.expires),
  };
  if (request.version) headers["If-Match"] = request.version;
  if (request.body !== undefined) headers["Content-Type"] = "application/json";
  return api(request.path, { method: request.method, headers, body: request.body });
}

async function startRequest(request) {
  if (model.pending) return;
  notice();
  remember(request);
  await checkPending(true);
}

async function checkPending(first = false) {
  const request = model.pending;
  if (!request || model.checking || !model.auth || !model.ready) return;
  model.checking = true;
  try {
    if (request.epoch !== model.system.host_epoch || request.device !== model.auth.device_id) {
      remember(null);
      notice("连接身份已更新。请检查历史确认上一条请求的结果，草稿已保留。");
      return;
    }
    if (first) await submit(request);
    let operation;
    try {
      operation = await api(`/operations/${request.id}`);
    } catch (error) {
      if (error.status !== 404) throw error;
      await submit(request);
      operation = await api(`/operations/${request.id}`);
    }
    if (operation.state === "failed") {
      remember(null);
      notice(operation.error.message);
    } else if (operation.state === "succeeded") {
      if (request.kind === "send" && operation.result.saved) {
        const sent = JSON.parse(request.body).text;
        if (stored(draftKey(request.session)) === sent) stored(draftKey(request.session), null);
        if (model.session?.id === request.session && $("draft").value === sent) $("draft").value = "";
        notice("已发送");
      } else if (request.kind === "create" && !model.session) {
        openChat(await api(`/sessions/${operation.result.session_id}`));
        notice("已新建会话");
      } else notice("已保存");
      remember(null);
    }
  } catch (error) {
    if ([400, 403, 409, 410, 412, 413, 422, 428].includes(error.status)) {
      remember(null);
      notice(error.message);
    } else {
      if (error.status !== 429 && error.status !== 503) connectivity(error);
      notice("正在核对请求结果。连接恢复后会继续查询，草稿已保留。");
    }
  } finally { model.checking = false; }
}

$("pair-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  if (!pairSecret) return;
  try {
    model.auth = await api("/auth/pair", {method: "POST", headers: {"Content-Type": "application/json"},
      body: JSON.stringify({secret: pairSecret, device_name: $("device").value})});
    pairSecret = null;
    model.ready = false;
    model.session = null;
    notice();
    show();
    refreshSoon();
  } catch (error) { notice(error.message); }
});

$("logout").addEventListener("click", async () => {
  if (model.pending && !window.confirm("有尚未确认的请求，确定要断开配对吗？")) return;
  try {
    await api("/auth/logout", {method: "POST", headers: {"X-CSRF-Token": model.auth.csrf_token}});
    connectivity(new HttpError(401, ""));
    notice("配对已断开。");
  } catch (error) { notice(error.message); }
});

function archivedTab(archived) {
  model.generation++;
  model.archived = archived;
  model.sessions = [];
  model.nextSession = null;
  $("active-tab").setAttribute("aria-pressed", String(!archived));
  $("archived-tab").setAttribute("aria-pressed", String(archived));
  renderSessions();
  refreshSoon();
}
$("active-tab").addEventListener("click", () => archivedTab(false));
$("archived-tab").addEventListener("click", () => archivedTab(true));
$("back").addEventListener("click", exitChat);
$("more-sessions").addEventListener("click", async () => {
  const generation = model.generation;
  try {
    const page = await api(sessionPath(model.nextSession));
    if (generation !== model.generation) return;
    const seen = new Set(model.sessions.map(({id}) => id));
    model.sessions.push(...page.items.filter(({id}) => !seen.has(id)));
    model.nextSession = page.next_cursor;
    renderSessions();
  } catch (error) { notice(error.message); }
});
$("older").addEventListener("click", async () => {
  const generation = model.generation;
  try {
    const page = await api(`/sessions/${model.session.id}/messages?cursor=${model.earlierCursor}`);
    if (generation !== model.generation) return;
    const scroll = $("messages-scroll");
    const height = scroll.scrollHeight;
    const position = scroll.scrollTop;
    model.history.unshift(...page.items);
    model.earlierCursor = page.next_cursor;
    renderMessages(page.transcript_state);
    scroll.scrollTop = position + scroll.scrollHeight - height;
  } catch (error) { notice(error.message); }
});
$("draft").addEventListener("input", () => {
  if (model.session) stored(draftKey(model.session.id), $("draft").value);
  controls();
});
$("composer").addEventListener("submit", (event) => {
  event.preventDefault();
  if ($("send").disabled) return;
  void startRequest(requestInfo(`/sessions/${model.session.id}/runs`, "send", {text: $("draft").value}, model.session.version));
});
$("stop").addEventListener("click", async () => {
  const {id, run_id} = model.session;
  if (!run_id || $("stop").disabled) return;
  $("stop").disabled = true;
  try {
    const request = requestInfo(`/sessions/${id}/runs/${encodeURIComponent(run_id)}/cancel`, "stop");
    await submit(request);
    for (let attempt = 0; attempt < 20; attempt++) {
      const operation = await api(`/operations/${request.id}`);
      if (operation.state === "failed") throw new Error(operation.error.message);
      if (operation.state === "succeeded") { notice("已请求停止本轮"); return; }
      await new Promise((resolve) => setTimeout(resolve, 150));
    }
    notice("已提交停止请求，请查看本轮状态。");
  } catch (error) { notice(error.message); }
  finally { controls(); refreshSoon(); }
});
$("new-session").addEventListener("click", () => {
  const options = model.system.engines.map(({id, name}) => new Option(name, id));
  $("engine").replaceChildren(...options);
  $("create-dialog").showModal();
});
$("create-form").addEventListener("submit", (event) => {
  event.preventDefault();
  $("create-dialog").close();
  void startRequest(requestInfo("/sessions", "create", {workspace_id: model.workspace.id, engine: $("engine").value}));
});
$("manage").addEventListener("click", async () => {
  try {
    const id = model.session.id;
    const metadata = await api(`/sessions/${id}/metadata`);
    if (model.session?.id !== id) return;
    model.metadata = metadata;
    $("session-title").value = metadata.title;
    $("pinned").checked = metadata.pinned;
    $("archived").checked = metadata.archived;
    $("manage-dialog").showModal();
  } catch (error) { notice(error.message); }
});
$("manage-form").addEventListener("submit", (event) => {
  event.preventDefault();
  const original = model.metadata;
  const patch = {};
  if ($("session-title").value.trim() !== original.title) patch.title = $("session-title").value.trim();
  if ($("pinned").checked !== original.pinned) patch.pinned = $("pinned").checked;
  if ($("archived").checked !== original.archived) patch.archived = $("archived").checked;
  $("manage-dialog").close();
  if (Object.keys(patch).length) void startRequest(requestInfo(`/sessions/${original.session_id}/metadata`, "metadata", patch, original.version, "PATCH"));
});
for (const button of document.querySelectorAll("[data-close]")) {
  button.addEventListener("click", () => button.closest("dialog").close());
}

document.addEventListener("visibilitychange", refreshSoon);
window.addEventListener("online", refreshSoon);
window.addEventListener("offline", () => online(false));
setInterval(refreshSoon, 2500);

async function boot() {
  try { model.pending = JSON.parse(stored("conduit-pending") || "null"); } catch { stored("conduit-pending", null); }
  try {
    if (!pairSecret) model.auth = await api("/auth/session");
  } catch (error) { connectivity(error); }
  show();
  refreshSoon();
}
void boot();
