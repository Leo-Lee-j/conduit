(function () {
  "use strict";
  const prototype = window.ConduitPrototype;

  function option(ctx, value, label, selected, disabled = false) {
    const selectedAttribute = selected ? " selected" : "";
    const disabledAttribute = disabled ? " disabled" : "";
    return `<option value="${ctx.escape(value)}"${selectedAttribute}${disabledAttribute}>${ctx.escape(label)}</option>`;
  }

  function row(ctx, id, label, help, control) {
    return `<div class="setting-row">
      <div class="setting-copy"><label class="setting-label" for="${id}">${ctx.escape(label)}</label>
        <p class="setting-description" id="${id}-help">${ctx.escape(help)}</p></div>
      <div class="setting-control">${control}</div>
    </div>`;
  }

  function toggle(ctx, id, label, help, checked, disabled) {
    const control = `<label class="switch" for="${id}">
      <input id="${id}" type="checkbox" role="switch" aria-describedby="${id}-help"${checked ? " checked" : ""}${disabled ? " disabled" : ""}>
      <span class="switch-track" aria-hidden="true"></span></label>`;
    return row(ctx, id, label, help, control);
  }

  function section(ctx, id, title, description, body, action = "") {
    return `<section class="settings-section" id="${id}" aria-labelledby="${id}-title">
      <div class="section-heading"><div>
        <h2 class="section-title" id="${id}-title">${ctx.escape(title)}</h2>
        <p class="section-description">${ctx.escape(description)}</p>
      </div>${action}</div>${body}</section>`;
  }

  function render(ctx) {
    const state = ctx.state;
    const engine = state.engine;
    const session = state.session;
    const disabled = ctx.isReadonly() ? " disabled" : "";
    const saved = state.defaults[engine] || { serviceId: null, modelId: null };
    const serviceId = saved.serviceId ?? null;
    const modelId = saved.modelId ?? null;
    const services = state.services.filter(service => service.engine === engine);
    const service = services.find(item => item.id === serviceId);
    const native = serviceId === null;
    const missingService = !native && !service;
    const models = service ? service.models : [];
    const missingModel = !!service && modelId !== null &&
      !models.some(model => model.id === modelId);

    let serviceOptions = option(ctx, "", "原生 CLI（无需添加服务）", native);
    if (missingService) {
      serviceOptions += option(ctx, serviceId, `已保存的服务不可用 · ${serviceId}`, true, true);
    }
    serviceOptions += services.map(item => option(ctx, item.id,
      `${item.name} · ${item.source === "ccs" ? "cc-switch" : "本地配置"}`, item.id === serviceId)).join("");

    let modelOptions;
    if (native) {
      modelOptions = option(ctx, modelId ?? "",
        modelId === null ? "由原生 CLI 选择模型" : `原生 CLI 已保存模型 · ${modelId}`, true);
    } else if (missingService) {
      modelOptions = option(ctx, modelId ?? "",
        modelId === null ? "请先重选可用服务" : `保留的模型选择 · ${modelId}`, true);
    } else {
      const defaultModel = models.find(model => model.id === service.defaultModel);
      const label = service.defaultModel
        ? `跟随服务默认 · ${defaultModel ? defaultModel.name : service.defaultModel}`
        : "跟随服务默认（未指定）";
      modelOptions = option(ctx, "", label, modelId === null);
      if (missingModel) {
        modelOptions += option(ctx, modelId, `已保存模型不在目录中 · ${modelId}`, true, true);
      }
      modelOptions += models.map(model => option(ctx, model.id,
        `${model.name} · ${model.id}`, model.id === modelId)).join("");
    }

    let defaultsBody = row(ctx, "sessions-service", "默认服务", "仅用于当前引擎的新会话。原生 CLI 是合法选项，不要求额外配置服务。",
      `<select class="control" id="sessions-service" aria-describedby="sessions-service-help"${disabled}>${serviceOptions}</select>`);
    const modelHelp = native
      ? "原生 CLI 使用自身模型目录；此处不伪造目录或模型选择器。"
      : "更换服务时跟随该服务默认模型；单独选择模型只改变新会话默认。";
    defaultsBody += row(ctx, "sessions-model", "默认模型", modelHelp,
      `<select class="control" id="sessions-model" aria-describedby="sessions-model-help"${ctx.isReadonly() || !service ? " disabled" : ""}>${modelOptions}</select>`);
    defaultsBody += `<p class="scope-note">新会话配置摘要：<span class="mono">${ctx.escape(ctx.currentRoute(engine))}</span></p>
      <p class="scope-note">这些默认值不改写已有会话的服务或模型。编辑共享服务本身，仍可能影响旧会话后续启动。</p>`;
    if (missingService || missingModel) {
      const message = missingService
        ? "已保存服务不可用，选择仍被保留。请明确重选；本原型不会静默回退或改写默认。"
        : "已保存模型不在当前示例目录中。选择仍被保留，可重选模型或跟随服务默认。";
      defaultsBody += `<p class="inline-note warning" role="status">${ctx.escape(message)}</p>`;
    }
    const defaults = section(ctx, "session-defaults", `新会话默认 · ${engine}`,
      `当前引擎：${engine}。仅配置该引擎的新会话；可用原生 CLI 无需设置持久默认。`, defaultsBody,
      `<button type="button" class="btn btn-quiet section-action" data-action="manage-services">管理模型与服务 ${ctx.icon("arrow-right")}</button>`);

    const permissionOptions = [["readonly", "只读"], ["workspace", "工作区写入"], ["full", "完全访问"]]
      .map(([value, label]) => option(ctx, value, label, session.permission === value)).join("");
    const permissionBody = row(ctx, "sessions-permission", "默认权限", "应用级默认，仅供新会话采用；已有会话继续保留原权限。",
      `<select class="control" id="sessions-permission" aria-describedby="sessions-permission-help sessions-permission-risk"${disabled}>${permissionOptions}</select>`) +
      `<p class="inline-note warning" id="sessions-permission-risk">完全访问可能允许工作区外的读写或高风险操作。不同引擎的实际授权由原生应用处理；此处只演示默认偏好，不执行授权，也不追改旧会话。</p>`;
    const permission = section(ctx, "session-permission", "默认权限",
      "权限与模型默认分开管理，避免将模型切换误解为授权变更。", permissionBody);

    const displayBody = toggle(ctx, "sessions-collapse-reasoning", "折叠推理过程",
      "即时影响聊天视图的推理展示，不改变模型输出或推理能力。", session.collapseReasoning, ctx.isReadonly()) +
      toggle(ctx, "sessions-collapse-questions", "折叠问题内容",
        "即时影响聊天视图的问题展示，不改变已经发送的内容。", session.collapseQuestions, ctx.isReadonly()) +
      `<p class="scope-note">显示偏好不是“仅新会话”。离线原型只演示开关状态，不连接实际聊天进程。</p>`;
    const display = section(ctx, "session-display", "显示与行为",
      "应用级显示偏好，即时影响聊天视图，与会话的服务、模型和权限无关。", displayBody);

    const namingDisabled = ctx.isReadonly() || !session.autoNaming;
    const namingOptions = [["first", "首轮命名"], ["latest", "随最新轮次更新"]]
      .map(([value, label]) => option(ctx, value, label, value === session.namingMode)).join("");
    const followHelp = session.namingFollow
      ? "跟随触发命名的会话自身的引擎、服务和模型，而非本页默认。此开关只演示规则，不执行真实命名。"
      : "已关闭跟随；本原型不配置或模拟独立命名模型，也不提供模型选择器。";
    let namingBody = toggle(ctx, "sessions-auto-naming", "自动命名",
      "开启后按命名策略处理会话名称；关闭时保留策略偏好。", session.autoNaming, ctx.isReadonly());
    namingBody += row(ctx, "sessions-naming-mode", "命名策略", "启用后生效。首轮用于首次命名；最新轮次可用于旧会话的后续轮次，不批量重命名历史会话。",
      `<select class="control" id="sessions-naming-mode" aria-describedby="sessions-naming-mode-help"${namingDisabled ? " disabled" : ""}>${namingOptions}</select>`);
    namingBody += toggle(ctx, "sessions-naming-follow", "跟随当前会话模型（规则演示）", followHelp, session.namingFollow, namingDisabled);
    let namingSummary = "自动命名已关闭；命名策略仍保留。";
    if (session.autoNaming) {
      const strategy = session.namingMode === "latest" ? "最新轮次可更新已有会话后续名称" : "首轮命名";
      const model = session.namingFollow ? "模型跟随触发命名的当前会话" : "不配置独立命名模型，本原型不执行命名";
      namingSummary = `${strategy}；${model}。`;
    }
    namingBody += `<p class="scope-note" id="sessions-naming-summary" aria-live="polite">命名摘要：${ctx.escape(namingSummary)}</p>
      <p class="scope-note">这里只演示规则和开关状态，不执行真实自动命名；不会批量修改历史名称或既有会话样本。</p>`;
    const naming = section(ctx, "session-naming", "自动命名", "可能作用于已有会话的后续轮次，不等同于新会话默认。", namingBody);
    const readonlyNote = ctx.isReadonly()
      ? `<p class="inline-note" role="status">只读模式：可通过公共栏切换引擎并浏览管理页，设置控件不可写入。</p>`
      : "";
    return readonlyNote + defaults + permission + display + naming;
  }

  function updateSession(ctx, id, key, value, message) {
    if (!ctx.isReadonly()) {
      ctx.state.session[key] = value;
      ctx.notify(message);
    }
    ctx.refresh({ focus: `#${id}` });
  }

  function bind(root, ctx) {
    const change = (id, callback) => {
      const input = root.querySelector(`#${id}`);
      if (input) input.addEventListener("change", () => callback(input));
    };
    const manage = root.querySelector('[data-action="manage-services"]');
    if (manage) manage.addEventListener("click", () => ctx.navigate("services"));

    change("sessions-service", input => {
      if (ctx.setDefault(ctx.state.engine, input.value === "" ? null : input.value, null)) {
        ctx.notify("示例新会话默认已更新；更换服务后跟随默认模型，已有会话保持不变。");
      }
      ctx.refresh({ focus: "#sessions-service" });
    });
    change("sessions-model", input => {
      const engine = ctx.state.engine;
      const saved = ctx.state.defaults[engine];
      const service = saved && ctx.state.services.find(item =>
        item.engine === engine && item.id === saved.serviceId);
      if (service && ctx.setDefault(engine, service.id, input.value === "" ? null : input.value)) {
        ctx.notify("示例新会话默认模型已更新；已有会话保持不变。");
      }
      ctx.refresh({ focus: "#sessions-model" });
    });
    change("sessions-permission", input => {
      if (["readonly", "workspace", "full"].includes(input.value)) {
        updateSession(ctx, input.id, "permission", input.value, "示例默认权限已更新，不追改已有会话权限。");
      } else ctx.refresh({ focus: "#sessions-permission" });
    });
    change("sessions-naming-mode", input => {
      if (ctx.state.session.autoNaming && ["first", "latest"].includes(input.value)) {
        updateSession(ctx, input.id, "namingMode", input.value, "示例命名策略已更新，不批量修改历史名称。");
      } else ctx.refresh({ focus: "#sessions-naming-mode" });
    });
    const switches = [
      ["sessions-collapse-reasoning", "collapseReasoning", "示例推理展示偏好已更新；聊天视图应即时采用。"],
      ["sessions-collapse-questions", "collapseQuestions", "示例问题展示偏好已更新；聊天视图应即时采用。"],
      ["sessions-auto-naming", "autoNaming", "示例自动命名开关已更新，不改写会话样本。"],
      ["sessions-naming-follow", "namingFollow", "示例命名跟随偏好已更新，不选择独立命名模型。"]
    ];
    switches.forEach(([id, key, message]) => change(id, input => {
      if (key === "namingFollow" && !ctx.state.session.autoNaming) {
        ctx.refresh({ focus: `#${id}` });
        return;
      }
      updateSession(ctx, id, key, input.checked, message);
    }));
  }

  prototype.pages.sessions = {
    title: "会话设置",
    summary: "按生效范围管理会话偏好。离线示例仅保存在内存，不运行 CLI 或改写既有会话。",
    search: [
      { id: "session-defaults", title: "新会话默认", keywords: "引擎 原生 CLI 默认服务 默认模型 Codex Claude Code OpenCode" },
      { id: "session-permission", title: "默认权限", keywords: "只读 readonly 工作区 workspace 完全访问 full 授权 风险" },
      { id: "session-display", title: "显示与行为", keywords: "折叠 推理 问题 collapseReasoning collapseQuestions 聊天视图" },
      { id: "session-naming", title: "自动命名", keywords: "自动命名 重命名 标题 首轮 first 最新 latest 当前会话 模型 跟随 namingFollow" }
    ],
    render,
    bind
  };
}());
