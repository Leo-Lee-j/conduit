"use strict";

const $ = (id) => document.getElementById(id);
const apiPath = "/api/remote/v1";
const stateLabels = {
  idle: "空闲", busy: "原生会话正在其他标签运行", pending_save: "正在保存消息",
  running: "运行中", waiting_input: "等待回答", waiting_approval: "等待审批",
  cancelling: "正在停止", cleanup_blocked: "电脑正在重新清理进程",
};
const sessionStateLabels = {
  idle: "空闲", busy: "其他标签运行", pending_save: "保存中",
  running: "运行中", waiting_input: "待回答", waiting_approval: "待审批",
  cancelling: "停止中", cleanup_blocked: "需清理",
};
const model = {
  auth: null, ready: false, system: null, workspace: null, session: null, approval: null, archived: false,
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
  syncApprovalControls();
}

function syncApprovalControls() {
  const disabled = !!model.pending || !!model.approval?.submitted
    || !model.online || !model.ready || !model.system?.writable || !model.session?.run_id;
  for (const control of $("approval").querySelectorAll("button, input")) control.disabled = disabled;
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
      const [session, messages, approvals] = await Promise.all([
        api(`/sessions/${id}`), api(`/sessions/${id}/messages`), api(`/sessions/${id}/approvals`),
      ]);
      if (generation !== model.generation) return;
      model.session = session;
      model.approval = approvals.items[0] || null;
      model.chatReady = messages.transcript_state === "ready";
      model.latest = messages.items;
      if (!model.history.length) model.earlierCursor = messages.next_cursor;
      $("title").textContent = session.title;
      $("state").textContent = session.engine + " · " + stateLabels[session.state];
      $("state").dataset.state = session.state;
      renderMessages(messages.transcript_state);
      renderApproval();
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
    info.className = "session-info";
    const title = document.createElement("strong");
    title.textContent = session.title;
    const meta = document.createElement("span");
    meta.className = "session-meta";
    const engine = document.createElement("span");
    engine.textContent = session.engine;
    meta.append(engine);
    if (session.pinned) {
      const pinned = document.createElement("span");
      pinned.className = "session-pin";
      pinned.textContent = "已置顶";
      meta.append(pinned);
    }
    const state = document.createElement("span");
    state.className = "session-state";
    state.dataset.state = session.state;
    state.textContent = sessionStateLabels[session.state] || "未知状态";
    const arrow = document.createElement("span");
    arrow.className = "arrow";
    arrow.setAttribute("aria-hidden", "true");
    info.append(title, meta);
    button.append(info, state, arrow);
    button.addEventListener("click", () => openChat(session));
    return button;
  });
  if (!items.length) {
    const empty = document.createElement("div");
    empty.className = "empty-state";
    const mark = document.createElement("span");
    mark.className = "empty-mark";
    mark.setAttribute("aria-hidden", "true");
    const title = document.createElement("strong");
    title.textContent = model.archived ? "没有已归档会话" : "从一段新会话开始";
    const description = document.createElement("p");
    description.textContent = model.archived ? "归档的会话会显示在这里" : "创建会话后，就能在手机上查看和继续桌面上的工作";
    empty.append(mark, title, description);
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
    const empty = document.createElement("div");
    empty.className = "empty-state empty-conversation";
    const mark = document.createElement("span");
    mark.className = "empty-mark";
    mark.setAttribute("aria-hidden", "true");
    const title = document.createElement("strong");
    const description = document.createElement("p");
    if (transcript === "failed") {
      title.textContent = "无法载入会话记录";
      description.textContent = "请确认电脑仍在运行，然后重试";
    } else if (transcript === "ready") {
      title.textContent = "这段会话还没有消息";
      description.textContent = "发送一条消息，继续桌面上的工作";
    } else {
      title.textContent = "正在载入会话";
      description.textContent = "正在同步桌面上的消息记录";
    }
    empty.append(mark, title, description);
    items.push(empty);
  }
  $("messages").replaceChildren(...items);
  $("older").hidden = !model.earlierCursor;
  if (bottom) scroll.scrollTop = scroll.scrollHeight;
}

function renderApproval() {
  const panel = $("approval");
  const approval = model.approval;
  const key = approval ? `${model.session?.run_id || ""}/${approval.id}/${approval.submitted}/${approval.supported}` : "";
  if (panel.dataset.key === key) {
    panel.hidden = !approval;
    syncApprovalControls();
    return;
  }
  panel.dataset.key = key;
  panel.replaceChildren();
  if (!approval) { panel.hidden = true; return; }
  panel.hidden = false;

  const title = document.createElement("h2");
  title.textContent = approval.kind === "question" ? "需要你回答" : "需要你审批";
  const summary = document.createElement("strong");
  summary.textContent = approval.title;
  panel.append(title, summary);
  if (approval.detail) {
    const detail = document.createElement("pre");
    detail.textContent = approval.detail;
    panel.append(detail);
  }
  if (approval.submitted) {
    const waiting = document.createElement("p");
    waiting.className = "muted";
    waiting.textContent = "已提交，正在等待电脑处理。";
    panel.append(waiting);
  } else if (!approval.supported) {
    const unsupported = document.createElement("p");
    unsupported.className = "muted";
    unsupported.textContent = "此问题格式暂不支持，请在电脑上完成。";
    panel.append(unsupported);
  } else if (approval.kind === "decision") {
    const actions = document.createElement("div");
    actions.className = "approval-actions";
    const deny = document.createElement("button");
    deny.type = "button";
    deny.textContent = "拒绝";
    deny.addEventListener("click", () => sendApproval({kind: "decision", allow: false}));
    actions.append(deny);
    if (approval.allow_once) {
      const allow = document.createElement("button");
      allow.type = "button";
      allow.className = "primary";
      allow.textContent = "允许一次";
      allow.addEventListener("click", () => sendApproval({kind: "decision", allow: true}));
      actions.append(allow);
    }
    panel.append(actions);
  } else {
    const form = document.createElement("form");
    form.className = "approval-form";
    for (const [index, question] of approval.questions.entries()) {
      const field = document.createElement("fieldset");
      const legend = document.createElement("legend");
      legend.textContent = question.header || question.question;
      field.append(legend);
      if (question.header && question.question !== question.header) {
        const prompt = document.createElement("p");
        prompt.textContent = question.question;
        field.append(prompt);
      }
      for (const option of question.options) {
        const label = document.createElement("label");
        label.className = "answer-option";
        const input = document.createElement("input");
        input.type = question.multiple ? "checkbox" : "radio";
        input.name = `answer-${index}`;
        input.value = option.label;
        input.dataset.questionId = question.id;
        label.append(input, document.createTextNode(option.label));
        if (option.description) {
          const description = document.createElement("small");
          description.textContent = option.description;
          label.append(description);
        }
        field.append(label);
      }
      if (question.custom) {
        const custom = document.createElement("input");
        custom.type = question.secret ? "password" : "text";
        custom.autocomplete = "off";
        custom.dataset.questionId = question.id;
        custom.dataset.customAnswer = "true";
        custom.placeholder = question.options.length ? "自定义答案（可选）" : "输入答案";
        custom.required = question.options.length === 0;
        if (!question.multiple && question.options.length) {
          for (const choice of field.querySelectorAll('input[type="radio"]')) {
            choice.addEventListener("change", () => { custom.value = ""; });
          }
          custom.addEventListener("input", () => {
            if (custom.value.trim()) for (const choice of field.querySelectorAll('input[type="radio"]')) choice.checked = false;
          });
        }
        field.append(custom);
      }
      form.append(field);
    }
    const actions = document.createElement("div");
    actions.className = "approval-actions";
    const skip = document.createElement("button");
    skip.type = "button";
    skip.textContent = "跳过";
    skip.addEventListener("click", () => sendApproval({kind: "decision", allow: false}));
    const answer = document.createElement("button");
    answer.type = "submit";
    answer.className = "primary";
    answer.textContent = "提交回答";
    actions.append(skip, answer);
    form.append(actions);
    form.addEventListener("submit", (event) => {
      event.preventDefault();
      const answers = Object.create(null);
      let valid = true;
      for (const question of approval.questions) {
        const values = [...form.querySelectorAll("input[data-question-id]")]
          .filter((input) => input.dataset.questionId === question.id)
          .filter((input) => input.dataset.customAnswer ? !!input.value.trim() : input.checked)
          .map((input) => input.dataset.customAnswer ? input.value.trim() : input.value);
        if (!values.length || (!question.multiple && values.length !== 1)
          || new Set(values).size !== values.length) valid = false;
        answers[question.id] = values;
      }
      if (!valid) { notice("请为每个问题提供有效答案。"); return; }
      void sendApproval({kind: "answer", answers}, true);
    });
    panel.append(form);
  }
  syncApprovalControls();
}

function sendApproval(body, sensitive = false) {
  const {id, run_id} = model.session || {};
  const approval = model.approval;
  if (!id || !run_id || !approval || model.pending) return;
  const path = `/sessions/${encodeURIComponent(id)}/runs/${encodeURIComponent(run_id)}/approvals/${encodeURIComponent(approval.id)}`;
  const request = requestInfo(path, body.kind === "answer" ? "answer" : "approval", body);
  request.approval = approval.id;
  request.run = run_id;
  if (sensitive) request.sensitive = true;
  void startRequest(request);
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
  $("state").textContent = session.engine + " · " + (stateLabels[session.state] || "空闲");
  $("state").dataset.state = session.state;
  show();
  renderMessages("loading", true);
  refreshSoon();
}

function exitChat() {
  model.generation++;
  model.session = null;
  model.approval = null;
  model.sessions = [];
  model.nextSession = null;
  $("sessions").replaceChildren();
  show();
  refreshSoon();
}

function remember(pending) {
  if (!pending && model.pending?.sensitive) model.pending.body = undefined;
  model.pending = pending;
  const persisted = pending?.sensitive ? {...pending, body: undefined, canRetry: false} : pending;
  stored("conduit-pending", persisted ? JSON.stringify(persisted) : null);
  controls();
}

function requestInfo(path, kind, body, version, method = "POST") {
  return {
    id: requestId(), epoch: model.system.host_epoch, device: model.auth.device_id,
    expires: Math.floor(Date.now() / 1000) + model.system.server_time - model.system.local_time + 540,
    path, kind, body: body === undefined ? undefined : JSON.stringify(body), version, method,
    session: model.session?.id,
  };
}

function requestId() {
  if (globalThis.crypto?.randomUUID) return globalThis.crypto.randomUUID();
  const bytes = new Uint8Array(16);
  try {
    globalThis.crypto?.getRandomValues(bytes);
    if (bytes.some(Boolean)) return [...bytes].map((byte) => byte.toString(16).padStart(2, "0")).join("");
  } catch { /* Request keys are identifiers, not authentication secrets. */ }
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}-${Math.random().toString(36).slice(2)}`;
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
    if (first && request.canRetry !== false) await submit(request);
    let operation;
    try {
      operation = await api(`/operations/${request.id}`);
    } catch (error) {
      if (error.status !== 404) throw error;
      if (request.canRetry === false) {
        remember(null);
        notice("回答结果尚未确认。请刷新请求状态后再决定是否重新回答。");
        return;
      }
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
      } else if (request.kind === "answer") {
        if (model.session?.id === request.session && model.session?.run_id === request.run
          && model.approval?.id === request.approval) {
          model.approval.submitted = true;
          renderApproval();
        }
        notice("已提交回答");
      } else if (request.kind === "approval") {
        if (model.session?.id === request.session && model.session?.run_id === request.run
          && model.approval?.id === request.approval) {
          model.approval.submitted = true;
          renderApproval();
        }
        notice("已提交审批");
      }
      else notice("已保存");
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
  } finally {
    model.checking = false;
    if (!model.pending) refreshSoon();
  }
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
