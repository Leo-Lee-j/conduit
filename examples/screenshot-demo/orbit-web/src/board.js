/* Fictional, offline demo data. No network or account required. */
const columns = [
  { id: "backlog", title: "待开始" },
  { id: "progress", title: "进行中" },
  { id: "review", title: "待验收" },
  { id: "done", title: "已完成" },
];
const initialTasks = [
  { id: 28, title: "设计项目邀请流程", description: "让新成员在第一次打开时，就知道从哪里开始。", tag: "设计", owner: "周宁", status: "backlog", date: "11 月 14 日" },
  { id: 29, title: "补充空状态引导", description: "用一句清晰的提示，帮助团队迈出第一步。", tag: "体验", owner: "林夏", status: "backlog", date: "11 月 15 日" },
  { id: 30, title: "整理键盘快捷键", description: "为高频操作留一条更短的路。", tag: "工程", owner: "陈一", status: "backlog", date: "11 月 16 日" },
  { id: 24, title: "实现任务筛选与搜索", description: "按负责人和关键词组合筛选，快速找到当前最重要的事。", tag: "工程", owner: "陈一", status: "progress", date: "11 月 11 日" },
  { id: 25, title: "打磨深色主题", description: "让长时间工作的界面更安静，层次更清晰。", tag: "设计", owner: "周宁", status: "progress", date: "11 月 12 日" },
  { id: 21, title: "完善任务详情交互", description: "查看目标、调整负责人，把进展留在任务里。", tag: "体验", owner: "林夏", status: "review", date: "11 月 10 日" },
  { id: 22, title: "适配移动端看板", description: "在小屏幕上保留同样清晰的信息优先级。", tag: "工程", owner: "陈一", status: "review", date: "11 月 10 日" },
  { id: 18, title: "确定产品视觉语言", description: "温润的中性色，搭配克制的松绿色。", tag: "设计", owner: "周宁", status: "done", date: "11 月 08 日" },
  { id: 19, title: "搭建项目基础结构", description: "让页面、样式和数据各就其位。", tag: "工程", owner: "陈一", status: "done", date: "11 月 09 日" },
];
const storageKey = "orbit-screenshot-demo-v1";
const owners = { "林夏": "teal", "周宁": "amber", "陈一": "blue" };
const $ = (selector) => document.querySelector(selector);
let tasks = initialTasks.map((task) => ({ ...task }));
try {
  const saved = JSON.parse(localStorage.getItem(storageKey));
  if (Array.isArray(saved) && saved.length && saved.every((task) =>
    Number.isSafeInteger(task.id) && typeof task.title === "string" &&
    typeof task.description === "string" && typeof task.date === "string" &&
    ["设计", "体验", "工程"].includes(task.tag) && Object.hasOwn(owners, task.owner) &&
    columns.some((column) => column.id === task.status))) tasks = saved;
  document.documentElement.dataset.theme = localStorage.getItem(`${storageKey}-theme`) === "dark" ? "dark" : "light";
} catch { /* file:// storage can be unavailable; keep the demo usable in memory. */ }
let editingId = null;
let toastTimer;

function textElement(tag, className, text) {
  const element = document.createElement(tag);
  element.className = className;
  element.textContent = text;
  return element;
}

function notify(message) {
  $("#toast").textContent = message;
  $("#toast").classList.add("visible");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => $("#toast").classList.remove("visible"), 2500);
}

function render() {
  const query = $("#search").value.trim().toLocaleLowerCase();
  const owner = $("#owner-filter").value;
  const visible = tasks.filter((task) =>
    `${task.title} ${task.description} ORB-${task.id}`.toLocaleLowerCase().includes(query) &&
    (owner === "all" || task.owner === owner));
  const board = $("#board");
  board.replaceChildren();
  for (const column of columns) {
    const section = document.createElement("section");
    section.className = `column ${column.id}`;
    const list = visible.filter((task) => task.status === column.id);
    const heading = textElement("div", "column-heading", "");
    heading.append(textElement("i", "status-dot", ""), textElement("h2", "", column.title), textElement("span", "count", list.length));
    const add = textElement("button", "add", "+");
    add.setAttribute("aria-label", `添加${column.title}任务`);
    add.addEventListener("click", () => openTask(null, column.id));
    heading.append(add);
    const cards = textElement("div", "cards", "");
    for (const task of list) {
      const card = textElement("button", "task-card", "");
      card.setAttribute("aria-label", `ORB-${task.id} ${task.title}，${task.owner}，${column.title}`);
      card.append(textElement("span", "task-id", `ORB-${task.id}`), textElement("h3", "", task.title), textElement("p", "", task.description));
      card.append(textElement("span", `tag ${task.tag === "设计" ? "design" : task.tag === "工程" ? "engineering" : ""}`, task.tag));
      const footer = textElement("div", "card-footer", "");
      const avatar = textElement("span", `avatar ${owners[task.owner]}`, task.owner[0]);
      avatar.title = task.owner;
      footer.append(textElement("time", "", task.date), avatar);
      card.append(footer);
      card.addEventListener("click", () => openTask(task));
      cards.append(card);
    }
    if (!list.length) cards.append(textElement("p", "column-empty", "暂时没有任务"));
    section.append(heading, cards);
    board.append(section);
  }
  const progress = Math.round(tasks.filter((task) => task.status === "done").length / tasks.length * 100);
  $("#progress-number").replaceChildren(document.createTextNode(progress), textElement("span", "", "%"));
  $("#progress-bar").style.width = `${progress}%`;
  $("#total-count").textContent = String(tasks.length).padStart(2, "0");
  $(".nav-count").textContent = String(tasks.length).padStart(2, "0");
  $("#active-count").textContent = String(tasks.filter((task) => task.status === "progress").length).padStart(2, "0");
  $("#empty-search").hidden = visible.length > 0;
}

function openTask(task = null, status = "backlog") {
  editingId = task?.id ?? null;
  $("#dialog-title").textContent = task ? `ORB-${task.id} · 任务详情` : "新建任务";
  $("#task-title").value = task?.title ?? "";
  $("#task-description").value = task?.description ?? "";
  $("#task-owner").value = task?.owner ?? "林夏";
  $("#task-status").value = task?.status ?? status;
  $("#task-dialog").showModal();
  $("#task-title").focus();
}

$("#task-form").addEventListener("submit", (event) => {
  event.preventDefault();
  const title = $("#task-title").value.trim();
  if (!title) { $("#task-title").setCustomValidity("请输入任务名称"); $("#task-title").reportValidity(); return; }
  const fields = { title, description: $("#task-description").value.trim(), owner: $("#task-owner").value, status: $("#task-status").value };
  if (editingId !== null) Object.assign(tasks.find((task) => task.id === editingId), fields);
  else tasks.push({ id: Math.max(...tasks.map((task) => task.id)) + 1, ...fields, tag: "体验", date: "待安排" });
  let persisted = true;
  try { localStorage.setItem(storageKey, JSON.stringify(tasks)); } catch { persisted = false; }
  $("#task-dialog").close();
  render();
  notify(persisted ? "已保存，下一步更清晰了。" : "已更新本次演示；浏览器未开放本地存储。");
});
$("#task-title").addEventListener("input", () => $("#task-title").setCustomValidity(""));
$("#close-dialog").addEventListener("click", () => $("#task-dialog").close());
$("#cancel-dialog").addEventListener("click", () => $("#task-dialog").close());
$("#new-task").addEventListener("click", () => openTask());
$("#search").addEventListener("input", render);
$("#owner-filter").addEventListener("change", render);
$("#theme-toggle").addEventListener("click", () => {
  const theme = document.documentElement.dataset.theme === "dark" ? "light" : "dark";
  document.documentElement.dataset.theme = theme;
  try { localStorage.setItem(`${storageKey}-theme`, theme); } catch { /* Optional preference. */ }
});
function showView() {
  const view = ["board", "milestones", "docs"].includes(location.hash.slice(1)) ? location.hash.slice(1) : "board";
  for (const name of ["board", "milestones", "docs"]) {
    $(`#${name}-view`).hidden = name !== view;
    const link = $(`[data-view="${name}"]`);
    link.classList.toggle("active", name === view);
    if (name === view) link.setAttribute("aria-current", "page");
    else link.removeAttribute("aria-current");
  }
}
window.addEventListener("hashchange", showView);
showView();
render();
