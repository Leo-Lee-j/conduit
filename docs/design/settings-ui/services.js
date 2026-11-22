(function () {
  'use strict';

  const inspectedModels = new Map();
  const probes = new Map();
  let sequence = 0;
  const key = service => JSON.stringify([service.engine, service.id]);
  const find = (ctx, id) => ctx.state.services.find(s => s.id === id && s.engine === ctx.state.engine);
  const sessionRefs = (ctx, id) => ctx.state.existingSessions.filter(s => s.serviceId === id);
  const defaultRefs = (ctx, id) => Object.entries(ctx.state.defaults).filter(([, route]) => route.serviceId === id);
  const route = ctx => ctx.state.defaults[ctx.state.engine] || { serviceId: null, modelId: null };
  const modelChoice = service => {
    const id = inspectedModels.get(key(service));
    return service.models.some(model => model.id === id) ? id : null;
  };
  const rowSelector = id => '#service-row-' + String(id).replace(/[^a-zA-Z0-9_-]/gu, c => String.fromCharCode(92) + c.codePointAt(0).toString(16) + ' ');

  function action(ctx, id, label, disabled, kind = 'btn-quiet') {
    const icons = { 'service-add': 'plus', 'service-test': 'refresh', 'service-back': 'arrow-left',
      'service-edit': 'settings', 'service-delete': 'x', 'service-set-default': 'check',
      'service-native-default': 'terminal', 'service-native-catalog': 'arrow-right' };
    return `<button type="button" id="${ctx.escape(id)}" data-action="${ctx.escape(id)}"
      class="btn ${ctx.escape(kind)}${id === 'service-back' ? ' service-back' : ''}"${disabled ? ' disabled' : ''}>
      ${ctx.icon(icons[id] || 'info')}${ctx.escape(label)}</button>`;
  }

  function stopProbe(job) {
    clearTimeout(job.timer);
    if (job.service.status === 'testing') job.service.status = job.previous;
    probes.delete(key(job.service));
  }

  function stopProbes(ctx, all = false) {
    for (const job of probes.values()) {
      if (all || ctx.isReadonly() || !ctx.isContextCurrent(job.token) || find(ctx, job.service.id) !== job.service) stopProbe(job);
    }
  }

  function verification(ctx, service) {
    const labels = { untested: '未模拟验证', testing: '模拟验证中', passed: '模拟验证通过', failed: '模拟验证失败' };
    const status = ['untested', 'testing', 'passed', 'failed'].includes(service.status) ? service.status : 'untested';
    return `<span class="status${status === 'passed' || status === 'failed' ? ' ' + ctx.escape(status) : ''}" data-status="${ctx.escape(status)}" role="status" aria-live="polite">
      ${ctx.icon(status === 'failed' ? 'alert' : status === 'passed' ? 'check' : 'info')}${ctx.escape(labels[status])}</span>`;
  }

  function serviceRow(ctx, service) {
    const e = value => ctx.escape(value);
    const selected = ctx.state.serviceView.selectedId === service.id;
    return `<button type="button" class="service-row" id="${e('service-row-' + service.id)}"
      data-action="service-browse" data-service-id="${e(service.id)}" aria-current="${selected}" aria-pressed="${selected}">
      <span class="setting-label">${e(service.name)}</span>
      <span class="scope-note">${e(service.source === 'ccs' ? 'cc-switch · 来源配置' : '本地 · 演示配置')}</span>
      ${route(ctx).serviceId === service.id ? `<span class="default-label">${ctx.icon('check')}新会话默认</span>` : ''}
      ${verification(ctx, service)}</button>`;
  }

  function serviceDetails(ctx, service) {
    const e = value => ctx.escape(value);
    const back = action(ctx, 'service-back', '返回服务列表', false);
    if (!service) return `${back}<div class="empty-state"><h2 id="service-detail-heading" tabindex="-1">选择一个服务</h2>
      <p>浏览服务不会改变新会话默认。请选择左侧或列表中的演示服务。</p></div>`;
    const sessions = sessionRefs(ctx, service.id);
    const defaults = defaultRefs(ctx, service.id);
    const local = service.source === 'local';
    const choice = modelChoice(service);
    const isDefault = route(ctx).serviceId === service.id;
    const sameRoute = isDefault && route(ctx).modelId === choice;
    const options = `<option value=""${choice === null ? ' selected' : ''}>${e('跟随服务默认 · ' + (service.defaultModel || '未设置'))}</option>` +
      service.models.map(model => `<option value="${e(model.id)}"${choice === model.id ? ' selected' : ''}>${e(model.name + ' · ' + model.id)}</option>`).join('');
    return `${back}<div class="section-heading"><div><h2 class="section-title" id="service-detail-heading" tabindex="-1">${e(service.name)}</h2>
      <p class="section-description">${e(local ? '本地演示服务 · 可独立编辑' : 'cc-switch 来源 · 不可作为本地配置编辑')}</p></div>
      ${isDefault ? `<span class="default-label">${ctx.icon('check')}当前新会话默认来源</span>` : '<span class="muted">仅在查看</span>'}</div>
      <div class="setting-copy"><span class="setting-label">服务端点</span>
      <p class="mono service-endpoint">${e(service.endpoint)}</p><p class="scope-note mono">${e(service.id)}</p></div>
      <p class="inline-note">${e(service.hasKey ? '已有演示密钥标记；原型不保存或显示密钥文本。' : '无演示密钥标记；原型不保存密钥文本。')}</p>
      <div class="setting-copy"><label class="setting-label" for="service-model">查看模型</label>
      <p class="setting-description">预选模型只用于查看，点击“设为新会话默认”才会应用。</p>
      <select id="service-model" class="control">${options}</select></div>
      <div class="service-actions">${action(ctx, 'service-set-default', sameRoute ? '已是此默认路由' : '设为新会话默认', ctx.isReadonly() || sameRoute, 'btn-primary')}</div>
      <p class="scope-note">仅影响当前引擎的新会话，不修改已有会话路由或历史。</p>
      <div class="setting-copy"><span class="setting-label">模拟验证</span>
      <p class="setting-description">只演示反馈流程，不访问端点，也不证明真实连接或生成能力。</p></div>
      <div class="service-actions">${verification(ctx, service)}${action(ctx, 'service-test', service.status === 'testing' ? '模拟验证中…' : '模拟验证', ctx.isReadonly() || service.status === 'testing')}</div>
      <div class="setting-copy"><span class="setting-label">已有会话关联 · ${e(sessions.length)}</span>
      <p class="setting-description">${e(sessions.length ? sessions.map(s => s.name + '（' + s.engine + '）').join('、') : '没有演示会话引用此服务。')}</p>
      <p class="scope-note">${e(defaults.length ? defaults.map(([engine]) => engine).join('、') + ' 的新会话默认也引用此服务。' : '没有新会话默认引用此服务。')}</p></div>
      ${sessions.length ? '<p class="inline-note warning">编辑共享服务可能影响已有会话未来使用，不会重写其历史或关联 ID。</p>' : ''}
      ${!local ? '<p class="inline-note">请在 cc-switch 来源应用管理配置；本原型不提供该来源的本地编辑或删除。</p>' : ''}
      <div class="service-actions">${action(ctx, 'service-edit', '编辑本地服务', ctx.isReadonly() || !local)}
      ${action(ctx, 'service-delete', '删除本地服务', ctx.isReadonly() || !local || sessions.length > 0 || defaults.length > 0, 'btn-danger')}</div>
      ${local && (sessions.length || defaults.length) ? '<p class="scope-note">此服务仍被会话或默认路由引用，删除已禁用。请先处理引用。</p>' : ''}`;
  }

  function render(ctx) {
    // Prune only this page's transient mock probes, never global catalog work.
    stopProbes(ctx);
    const e = value => ctx.escape(value);
    const services = ctx.state.services.filter(s => s.engine === ctx.state.engine);
    const selected = find(ctx, ctx.state.serviceView.selectedId);
    const current = route(ctx);
    const source = services.find(s => s.id === current.serviceId);
    const unavailable = current.serviceId !== null && (!source || (current.modelId !== null && !source.models.some(m => m.id === current.modelId)));
    const groups = [['local', '本地服务'], ['ccs', 'cc-switch 来源']].map(([origin, title]) => {
      const items = services.filter(s => s.source === origin);
      return `<section class="settings-section"><div class="section-heading"><h3 class="section-title">${e(title)} · ${e(items.length)}</h3></div>
        ${items.length ? items.map(s => serviceRow(ctx, s)).join('') : '<p class="empty-state">当前引擎没有此来源的演示服务。</p>'}</section>`;
    }).join('');
    return `<section class="settings-section" id="services-route" tabindex="-1">
      <div class="section-heading"><div><h2 class="section-title">新会话默认路由</h2>
      <p class="section-description">默认、查看对象和模拟验证是三个独立状态。</p></div>
      <div class="section-action">${action(ctx, 'service-add', '添加本地服务', ctx.isReadonly(), 'btn-primary')}</div></div>
      <div class="setting-row"><div class="setting-copy"><span class="setting-label mono">${e(ctx.currentRoute(ctx.state.engine))}</span>
      <p class="setting-description">仅影响该引擎的新会话；所有配置和模型均为演示数据。</p></div>
      <div class="setting-control">${action(ctx, 'service-native-default', '将原生 CLI 设为默认', ctx.isReadonly() || (current.serviceId === null && current.modelId === null))}</div></div>
      <p class="inline-note">${e('查看对象：' + (selected ? selected.name + ' · ' + (selected.source === 'ccs' ? 'cc-switch 来源' : '本地服务') : '尚未选择服务'))}</p>
      ${unavailable ? '<p class="inline-note warning" role="status">默认来源或模型不可用。请浏览有效服务并明确重选；原型不会静默改写默认，也不模拟启动回退。</p>' : ''}
      ${ctx.isReadonly() ? '<p class="scope-note">只读模式：仍可浏览、预选模型和切换引擎；配置写入及模拟验证已禁用。</p>' : ''}
      </section>
      <div class="service-layout" id="services-browse" tabindex="-1">
      <div class="service-list" data-service-list aria-label="按来源分组的演示服务">${groups}</div>
      <section class="service-detail settings-section" data-service-detail aria-label="服务详情">${serviceDetails(ctx, selected)}</section></div>
      <section class="settings-section" id="services-native" tabindex="-1"><div class="section-heading"><h2 class="section-title">原生 CLI 模型目录</h2></div>
      <p class="section-description">原生 CLI 由引擎自身配置维护，不是已保存的本地服务，也不属于 cc-switch 来源列表。</p>
      <div class="section-action">${action(ctx, 'service-native-catalog', '查看原生目录说明', false)}</div></section>`;
  }

  function input(ctx, name, label, value, type = 'text', attributes = '') {
    const e = value => ctx.escape(value);
    return `<div class="setting-row"><label class="setting-label" for="service-dialog-${e(name)}">${e(label)}</label>
      <input class="control" id="service-dialog-${e(name)}" name="${e(name)}" type="${e(type)}" value="${e(value)}" ${attributes}></div>`;
  }

  function openEditor(ctx, service) {
    if (ctx.isReadonly() || (service && service.source !== 'local')) return;
    const e = value => ctx.escape(value);
    const engine = ctx.state.engine;
    const id = service ? service.id : null;
    const sessions = service ? sessionRefs(ctx, id) : [];
    stopProbes(ctx, true);
    ctx.refresh();
    const token = ctx.contextToken();
    const content = input(ctx, 'name', '服务名称', service ? service.name : '', 'text', 'required maxlength="80" autocomplete="off"') +
      input(ctx, 'endpoint', '演示端点 · http(s)', service ? service.endpoint : 'https://api.example.com/v1', 'url', 'required autocomplete="off"') +
      `<div class="setting-row"><div class="setting-copy"><label class="setting-label" for="service-dialog-models">模型 ID · 每行一个</label>
      <p class="setting-description">仅维护演示目录；移除模型不会自动修复引用它的新会话默认。</p></div>
      <textarea id="service-dialog-models" name="models" class="control mono" rows="4" required>${e(service ? service.models.map(m => m.id).join('\n') : 'demo-model')}</textarea></div>` +
      input(ctx, 'defaultModel', '服务自身默认模型 ID', service ? service.defaultModel || '' : 'demo-model', 'text', 'autocomplete="off"') +
      `<div class="setting-row"><div class="setting-copy"><label class="setting-label" for="service-dialog-apiKey">API Key · 演示输入</label>
      <p class="setting-description">${e(service && service.hasKey ? '已有密钥标记，留空保留。仅记录是否填写，不保存文本。' : '只记录是否填写，不保存密钥文本。')}</p></div>
      <div class="setting-control"><input type="password" id="service-dialog-apiKey" name="apiKey" class="control" autocomplete="off" spellcheck="false" value="">
      <button type="button" class="btn btn-quiet" data-action="service-reveal" aria-controls="service-dialog-apiKey" aria-pressed="false">${ctx.icon('eye')}<span>显示密钥</span></button></div></div>` +
      (service && service.hasKey ? '<label class="setting-row"><span class="setting-label">移除已有演示密钥标记</span><input type="checkbox" name="removeKey"></label>' : '') +
      (sessions.length ? `<label class="setting-row inline-note warning" for="service-dialog-impact"><span class="setting-copy"><span class="setting-label">确认共享配置影响</span>
      <span class="setting-description">${e('此服务被 ' + sessions.length + ' 个已有会话引用：' + sessions.map(s => s.name).join('、') + '。保存将更新同一服务 ID，可能影响它们未来使用；不会重写历史或会话路由。')}</span></span>
      <input id="service-dialog-impact" type="checkbox" name="confirmShared" required></label>` : '');
    ctx.openDialog({
      title: service ? '编辑本地服务' : '添加本地服务',
      description: '仅修改离线原型内存，不访问端点。编辑共享服务不创建版本；保存不会自动设为新会话默认。',
      content,
      onMount(body) {
        const secret = body.querySelector('#service-dialog-apiKey');
        const reveal = body.querySelector('[data-action="service-reveal"]');
        reveal.addEventListener('click', () => {
          const visible = secret.type === 'password';
          secret.type = visible ? 'text' : 'password';
          reveal.setAttribute('aria-pressed', String(visible));
          reveal.querySelector('span').textContent = visible ? '隐藏密钥' : '显示密钥';
        });
      },
      onSubmit(data, body) {
        if (ctx.isReadonly()) return '当前为只读模式，未保存配置。';
        if (!ctx.isContextCurrent(token) || ctx.state.engine !== engine) return '查看上下文已变化，请取消后重新打开编辑。';
        const current = id === null ? null : find(ctx, id);
        if (id !== null && (!current || current.source !== 'local')) return '服务已不可本地编辑，请重新查看来源。';
        const name = String(data.get('name') || '').trim();
        const endpoint = String(data.get('endpoint') || '').trim();
        if (!name || name.length > 80) return '请填写不超过 80 个字符的服务名称。';
        try {
          const url = new URL(endpoint);
          if (!['http:', 'https:'].includes(url.protocol) || !url.hostname || url.username || url.password) return '端点须为不含账号或密钥的 http(s) 地址。';
        } catch (_) { return '请填写有效的 http(s) 演示端点。'; }
        const ids = [...new Set(String(data.get('models') || '').split(/\r?\n/).map(v => v.trim()).filter(Boolean))];
        if (!ids.length || ids.length > 50 || ids.some(v => /\s/.test(v) || v.length > 120)) return '请填写 1–50 个模型 ID，每行一个，不含空格且不超过 120 个字符。';
        const defaultModel = String(data.get('defaultModel') || '').trim() || ids[0];
        if (!ids.includes(defaultModel)) return '服务默认模型必须存在于本次模型目录中。';
        if (id !== null && sessionRefs(ctx, id).length && !data.get('confirmShared')) return '请确认共享配置影响；若引用已变化，请取消后重新打开编辑。';
        let savedId = id;
        if (savedId === null) do { savedId = 'local-demo-' + (++sequence); } while (ctx.state.services.some(s => s.id === savedId));
        const oldModels = new Map((current ? current.models : []).map(m => [m.id, m.name]));
        const hasKey = Boolean(String(data.get('apiKey') || '').trim()) || Boolean(current && current.hasKey && !data.get('removeKey'));
        const saved = { id: savedId, name, engine, source: 'local', endpoint, defaultModel,
          models: ids.map(modelId => ({ id: modelId, name: oldModels.get(modelId) || modelId })), status: 'untested', hasKey };
        if (current) ctx.state.services.splice(ctx.state.services.indexOf(current), 1, saved);
        else ctx.state.services.push(saved);
        ctx.state.serviceView.selectedId = savedId;
        ctx.state.serviceView.detailOpen = true;
        data.delete('apiKey');
        body.querySelector('#service-dialog-apiKey').value = '';
        ctx.notify(current ? '共享服务配置已更新。原默认路由与已有会话记录保持不变。' : '本地演示服务已添加，尚未设为新会话默认。');
        return '';
      }
    });
  }

  function openDelete(ctx, service) {
    if (!service || ctx.isReadonly() || service.source !== 'local') return;
    if (sessionRefs(ctx, service.id).length || defaultRefs(ctx, service.id).length) {
      ctx.notify('服务仍被已有会话或新会话默认引用，不能删除。', 'error');
      return;
    }
    stopProbes(ctx, true);
    ctx.refresh();
    const token = ctx.contextToken();
    ctx.openDialog({ title: '删除本地演示服务', description: '只有没有会话及默认引用的本地服务才能删除。此操作仅修改原型内存。',
      submitLabel: '删除服务', content: input(ctx, 'confirmation', '确认删除，请输入：' + service.name, '', 'text', 'required autocomplete="off"'),
      onSubmit(data) {
        if (ctx.isReadonly()) return '当前为只读模式，未删除服务。';
        if (!ctx.isContextCurrent(token)) return '查看上下文已变化，请取消后重新检查服务。';
        const live = find(ctx, service.id);
        if (!live || live.source !== 'local') return '此来源不可本地删除。';
        if (sessionRefs(ctx, live.id).length || defaultRefs(ctx, live.id).length) return '服务仍被已有会话或默认引用，不能删除。';
        if (String(data.get('confirmation') || '').trim() !== live.name) return '服务名称不匹配，请重新确认。';
        ctx.state.services.splice(ctx.state.services.indexOf(live), 1);
        inspectedModels.delete(key(live));
        ctx.state.serviceView.selectedId = null;
        ctx.state.serviceView.detailOpen = false;
        ctx.notify('未被引用的本地演示服务已删除。');
        return '';
      }
    });
  }

  function mockTest(ctx, service) {
    if (!service || ctx.isReadonly() || service.status === 'testing') return;
    const job = { service, previous: service.status, token: ctx.contextToken(), timer: null };
    const fail = ctx.state.scenario === 'verification-failure';
    probes.set(key(service), job);
    service.status = 'testing';
    ctx.refresh({ focus: '#service-test' });
    job.timer = setTimeout(() => {
      if (probes.get(key(service)) !== job) return;
      if (!ctx.isContextCurrent(job.token) || ctx.isReadonly() || find(ctx, service.id) !== service) { stopProbe(job); return; }
      probes.delete(key(service));
      service.status = fail ? 'failed' : 'passed';
      ctx.refresh();
      ctx.notify(fail ? '模拟验证失败：演示失败反馈；未访问网络，也未修改默认。' : '模拟验证通过：仅为演示结果，不代表真实连接或生成能力。', fail ? 'error' : 'success');
    }, 900);
  }

  function bind(root, ctx) {
    const token = ctx.contextToken();
    const selectedId = ctx.state.serviceView.selectedId;
    const picker = root.querySelector('#service-model');
    if (picker) picker.addEventListener('change', event => {
      if (!ctx.isContextCurrent(token) || ctx.state.serviceView.selectedId !== selectedId) return;
      const service = find(ctx, ctx.state.serviceView.selectedId);
      if (!service) return;
      inspectedModels.set(key(service), event.target.value || null);
      ctx.refresh({ focus: '#service-model' });
    });
    root.addEventListener('click', event => {
      const button = event.target.closest('button[data-action]');
      if (!button || !root.contains(button) || button.disabled || !ctx.isContextCurrent(token) || ctx.state.serviceView.selectedId !== selectedId) return;
      const service = find(ctx, button.dataset.serviceId || ctx.state.serviceView.selectedId);
      switch (button.dataset.action) {
        case 'service-browse':
          if (!service) return;
          ctx.state.serviceView.selectedId = service.id;
          ctx.state.serviceView.detailOpen = true;
          ctx.refresh({ focus: '#service-detail-heading' });
          break;
        case 'service-back':
          ctx.state.serviceView.detailOpen = false;
          ctx.refresh({ focus: service ? rowSelector(service.id) : '#service-add' });
          break;
        case 'service-add': openEditor(ctx, null); break;
        case 'service-edit': if (service) openEditor(ctx, service); break;
        case 'service-delete': openDelete(ctx, service); break;
        case 'service-test': mockTest(ctx, service); break;
        case 'service-native-catalog':
          ctx.notify('原生目录由引擎配置维护。本离线原型不读取 CLI 文件或访问外部目录。', 'info');
          break;
        case 'service-native-default':
        case 'service-set-default': {
          if (ctx.isReadonly()) return;
          const native = button.dataset.action === 'service-native-default';
          if (!native && !service) return;
          const select = root.querySelector('#service-model');
          const modelId = native ? null : (select ? select.value || null : modelChoice(service));
          if (ctx.setDefault(ctx.state.engine, native ? null : service.id, modelId)) {
            ctx.notify('该引擎的新会话默认已更新；已有会话路由与记录保持不变。');
            ctx.refresh({ focus: native ? '#services-route' : '#service-model' });
          } else ctx.notify('默认设置未更新，请检查只读状态及服务或模型是否可用。', 'error');
          break;
        }
      }
    });
  }

  window.ConduitPrototype.pages.services = {
    title: '模型与服务',
    summary: '管理演示服务与新会话默认，分清浏览、共享配置和模拟验证',
    search: [
      { id: 'services-route', title: '新会话默认路由', keywords: '引擎 默认 来源 路由 Codex Claude OpenCode' },
      { id: 'services-browse', title: '服务、模型与模拟验证', keywords: '本地 cc-switch CCS 模型 API Key 密钥 添加 编辑 删除 关联 连接 验证' },
      { id: 'services-native', title: '原生 CLI 模型目录', keywords: '原生 CLI 模型 目录 配置' }
    ],
    render,
    bind
  };
}());
