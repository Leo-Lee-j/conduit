(function () {
  'use strict';

  const UI_FONTS = [
    ['system', '系统默认'],
    ['Segoe UI', 'Segoe UI'],
    ['Microsoft YaHei UI', 'Microsoft YaHei UI（微软雅黑）'],
    ['Consolas', 'Consolas']
  ];
  const CODE_FONTS = [
    ['mono', '系统等宽字体'], ['Consolas', 'Consolas'],
    ['Cascadia Mono', 'Cascadia Mono'], ['Courier New', 'Courier New']
  ];
  const FAMILIES = {
    system: 'system-ui, "Segoe UI", "Microsoft YaHei UI", sans-serif',
    'Segoe UI': '"Segoe UI", "Microsoft YaHei UI", sans-serif',
    'Microsoft YaHei UI': '"Microsoft YaHei UI", "Segoe UI", sans-serif',
    Consolas: 'Consolas, "Microsoft YaHei UI", monospace',
    mono: 'Consolas, "Cascadia Mono", "Courier New", monospace',
    'Cascadia Mono': '"Cascadia Mono", Consolas, monospace',
    'Courier New': '"Courier New", Consolas, monospace'
  };
  const MODES = [['light', '浅色'], ['dark', '深色'], ['system', '跟随系统']];
  const SELECTS = { mode: MODES, uiFont: UI_FONTS, chatFont: UI_FONTS, codeFont: CODE_FONTS };
  const RANGES = {
    uiSize: { min: 12, max: 20, step: 1, fallback: 16, unit: 'px' },
    chatSize: { min: 12, max: 20, step: 1, fallback: 15, unit: 'px' },
    chatLineHeight: { min: 1.2, max: 2, step: 0.1, fallback: 1.5, unit: '倍' },
    codeSize: { min: 12, max: 20, step: 1, fallback: 13, unit: 'px' }
  };
  const SAMPLES = [
    {
      id: 'narrow', title: '本轮评审 · 窄窗口',
      user: '技术希望大字号+长英文+右面板展开的窄主区规格。',
      answer: '先将服务列表＋详情改为交替单页，保留文字导航；单页仍不满足最小内容宽，才折叠分类。'
    },
    {
      id: 'defaults', title: '本轮评审 · 新会话默认',
      user: '浏览服务的选中态与新会话默认态，在宽/窄页如何明确区分？',
      answer: '默认：勾＋“新会话默认”；宽窄页均保留“默认：B”摘要，查看 A 不改变 B。'
    }
  ];
  const CODE_SAMPLE = [
    'function applyUiSize(ctx, value) {',
    '  if (ctx.isReadonly()) return;',
    '  ctx.state.appearance.uiSize =',
    '    Math.min(20, Math.max(12, Number(value)));',
    '  ctx.applyAppearance();',
    '  // 仅更新内存演示；不重建编辑器，不写入配置文件。',
    '}'
  ].join('\n');
  let previewSampleId = SAMPLES[0].id;

  function fieldId(key) {
    return 'appearance-' + key.replace(/[A-Z]/g, letter => '-' + letter.toLowerCase());
  }

  function numeric(key, value) {
    const spec = RANGES[key];
    const number = Number(value);
    const bounded = Math.min(spec.max, Math.max(spec.min, Number.isFinite(number) ? number : spec.fallback));
    return Number((Math.round(bounded / spec.step) * spec.step).toFixed(1));
  }

  function row(ctx, key, label, description, control) {
    const e = ctx.escape, id = fieldId(key);
    return `<div class="setting-row">
      <div class="setting-copy"><label class="setting-label" for="${e(id)}">${e(label)}</label>
        <p class="setting-description" id="${e(id)}-note">${e(description)}</p></div>
      <div class="setting-control">${control}</div>
    </div>`;
  }

  function select(ctx, key, label, description) {
    const e = ctx.escape, id = fieldId(key), value = ctx.state.appearance[key];
    const options = SELECTS[key].map(([item, title]) =>
      `<option value="${e(item)}"${item === value ? ' selected' : ''}>${e(title)}</option>`).join('');
    return row(ctx, key, label, description,
      `<select class="control" id="${e(id)}" data-appearance-key="${e(key)}"
        aria-describedby="${e(id)}-note"${ctx.isReadonly() ? ' disabled' : ''}>${options}</select>`);
  }

  function range(ctx, key, label, description) {
    const e = ctx.escape, id = fieldId(key), spec = RANGES[key];
    const value = numeric(key, ctx.state.appearance[key]);
    return row(ctx, key, label, description,
      `<input class="control" type="range" id="${e(id)}" data-appearance-key="${e(key)}"
        min="${e(spec.min)}" max="${e(spec.max)}" step="${e(spec.step)}" value="${e(value)}"
        aria-describedby="${e(id)}-note" aria-valuetext="${e(value)} ${e(spec.unit)}"
        ${ctx.isReadonly() ? 'disabled' : ''}>
      <output class="status" id="${e(id)}-value" for="${e(id)}" aria-live="polite">${e(value)} ${e(spec.unit)}</output>`);
  }

  function section(ctx, id, title, description, content, action) {
    const e = ctx.escape;
    return `<section class="settings-section" id="${e(id)}" tabindex="-1" aria-labelledby="${e(id)}-title">
      <div class="section-heading"><div><h2 class="section-title" id="${e(id)}-title">${e(title)}</h2>
        <p class="section-description">${e(description)}</p></div>
        ${action ? `<div class="section-action">${action}</div>` : ''}</div>${content}
    </section>`;
  }

  function conversation(ctx) {
    const e = ctx.escape, sample = SAMPLES.find(item => item.id === previewSampleId) || SAMPLES[0];
    const options = SAMPLES.map(item =>
      `<option value="${e(item.id)}"${item.id === sample.id ? ' selected' : ''}>${e(item.title)}</option>`).join('');
    return `<article class="preview-card" aria-labelledby="appearance-conversation-title">
      <div class="preview-header"><strong id="appearance-conversation-title">${ctx.icon('message')} 会话排版预览</strong>
        <label class="setting-label" for="appearance-preview-sample">预览片段</label>
        <select class="control control-small" id="appearance-preview-sample">${options}</select></div>
      <div class="preview-body" data-chat-preview>
        <p><strong>用户</strong><br><span data-preview-user>${e(sample.user)}</span></p>
        <p><strong>助手</strong><br><span data-preview-answer>${e(sample.answer)}</span></p>
      </div><p class="scope-note">取自本轮设置评审的静态片段；不读取或修改已有会话。</p>
    </article>`;
  }

  function codePreview(ctx) {
    return `<article class="preview-card" aria-labelledby="appearance-code-preview-title">
      <div class="preview-header"><strong id="appearance-code-preview-title">${ctx.icon('terminal')} JavaScript 代码预览</strong>
        <span class="status">静态示例 · 不执行</span></div>
      <pre class="preview-body"><code class="mono" data-code-preview>${ctx.escape(CODE_SAMPLE)}</code></pre>
      <p class="scope-note">代码字体与聊天字体独立；长行会换行，不撑宽设置主区。</p>
    </article>`;
  }

  function updatePreviews(root, ctx) {
    const appearance = ctx.state.appearance;
    const chat = root.querySelector('[data-chat-preview]');
    if (chat) {
      chat.style.fontFamily = FAMILIES[appearance.chatFont] || FAMILIES.system;
      chat.style.fontSize = numeric('chatSize', appearance.chatSize) + 'px';
      chat.style.lineHeight = String(numeric('chatLineHeight', appearance.chatLineHeight));
      chat.style.overflowWrap = 'anywhere';
    }
    const code = root.querySelector('[data-code-preview]');
    if (code) {
      code.style.fontFamily = FAMILIES[appearance.codeFont] || FAMILIES.mono;
      code.style.fontSize = numeric('codeSize', appearance.codeSize) + 'px';
      code.style.lineHeight = '1.6';
      code.parentElement.style.whiteSpace = 'pre-wrap';
      code.parentElement.style.overflowWrap = 'anywhere';
    }
    Object.keys(RANGES).forEach(key => {
      const id = fieldId(key), input = root.querySelector('#' + id);
      const output = root.querySelector('#' + id + '-value');
      const value = numeric(key, appearance[key]), text = value + ' ' + RANGES[key].unit;
      if (input) { input.value = String(value); input.setAttribute('aria-valuetext', text); }
      if (output) output.textContent = text;
    });
  }

  window.ConduitPrototype.pages.appearance = {
    title: '外观与字体',
    summary: '界面、聊天与代码独立调节；在会话与代码内容中预览效果。',
    search: [
      { id: 'appearance-theme', title: '主题', keywords: '外观 浅色 深色 跟随系统 light dark system 纯色背景' },
      { id: 'appearance-interface', title: '界面字体与字号', keywords: 'UI 字体 字号 12 20 恢复默认 Segoe Microsoft YaHei' },
      { id: 'appearance-chat', title: '聊天排版与预览', keywords: '聊天 会话 字体 字号 行高 Conversation Line Height 预览' },
      { id: 'appearance-code', title: '代码字体与预览', keywords: '代码 等宽 字体 字号 Consolas Cascadia Courier 预览' }
    ],

    render(ctx) {
      const readonly = ctx.isReadonly();
      const notice = readonly ? '只读模式：设置不可修改，仍可切换会话片段与查看预览。'
        : '所有更改仅保留在本次内存演示，不保存配置、不下载字体。';
      const reset = `<button type="button" class="btn btn-quiet" id="appearance-ui-reset"
        ${readonly ? 'disabled' : ''} aria-describedby="appearance-ui-reset-note">${ctx.icon('refresh')} 恢复界面默认</button>`;
      return `<div class="appearance-layout"><div class="appearance-form">
        <p class="inline-note">${ctx.escape(notice)}</p>` +
        section(ctx, 'appearance-theme', '主题', '沿用主题纯色背景，不添加装饰图片。',
          select(ctx, 'mode', '主题模式', '跟随系统会自动响应本机浅色或深色外观。')) +
        section(ctx, 'appearance-interface', '界面字体与字号', '只影响工作台界面，不联动聊天或代码字体。',
          select(ctx, 'uiFont', '界面字体', '仅使用本机字体；未安装时回退，不远程下载。') +
          range(ctx, 'uiSize', '界面字号', '12–20 px。即时应用；大字号与长标签由公共布局换行。') +
          '<p class="scope-note" id="appearance-ui-reset-note">恢复默认只重置界面字体与字号，不改变主题、聊天、代码或背景。</p>', reset) +
        section(ctx, 'appearance-chat', '聊天排版', '调整会话正文，直接对照本轮评审的真实内容片段。',
          select(ctx, 'chatFont', '聊天字体', '只改变聊天正文，代码仍使用独立的字体设置。') +
          range(ctx, 'chatSize', '聊天字号', '12–20 px。拖动或使用方向键，预览即时更新。') +
          range(ctx, 'chatLineHeight', '聊天文字行高（Conversation Line Height）', '1.2–2.0 倍；不会修改代码行高。')) +
        section(ctx, 'appearance-code', '代码排版', '独立调节等宽字体与字号，不重建或内嵌编辑器。',
          select(ctx, 'codeFont', '代码字体', '优先使用本机等宽字体；不可用时回退到系统等宽字体。') +
          range(ctx, 'codeSize', '代码字号', '12–20 px。这里只预览排版，不执行示例代码。')) +
        `</div><aside class="appearance-preview-pane" aria-label="排版预览">
          <div class="section-heading"><h2 class="section-title">排版预览</h2></div>
          ${conversation(ctx)}${codePreview(ctx)}
        </aside></div>`;
    },

    bind(root, ctx) {
      updatePreviews(root, ctx);
      const previewSelect = root.querySelector('#appearance-preview-sample');
      previewSelect.addEventListener('change', () => {
        const sample = SAMPLES.find(item => item.id === previewSelect.value) || SAMPLES[0];
        previewSampleId = sample.id;
        ctx.refresh({ focus: '#appearance-preview-sample' });
      });

      root.querySelectorAll('select[data-appearance-key]').forEach(control => {
        control.addEventListener('change', () => {
          const key = control.dataset.appearanceKey;
          if (ctx.isReadonly()) {
            control.value = ctx.state.appearance[key];
            ctx.notify('只读模式下不能修改设置。', 'info');
            return;
          }
          if (!SELECTS[key].some(([value]) => value === control.value)) return;
          ctx.state.appearance[key] = control.value;
          ctx.applyAppearance();
          ctx.refresh({ focus: '#' + control.id });
          ctx.notify('已应用到当前内存演示。', 'success');
        });
      });

      root.querySelectorAll('input[data-appearance-key]').forEach(control => {
        const apply = () => {
          if (ctx.isReadonly()) { updatePreviews(root, ctx); return false; }
          const key = control.dataset.appearanceKey;
          ctx.state.appearance[key] = numeric(key, control.value);
          ctx.applyAppearance();
          updatePreviews(root, ctx);
          return true;
        };
        control.addEventListener('input', apply);
        control.addEventListener('change', () => {
          if (apply()) ctx.notify('排版已应用，仅保留在当前演示。', 'success');
        });
      });

      root.querySelector('#appearance-ui-reset').addEventListener('click', () => {
        if (ctx.isReadonly()) { ctx.notify('只读模式下不能恢复设置。', 'info'); return; }
        ctx.state.appearance.uiFont = 'system';
        ctx.state.appearance.uiSize = 16;
        ctx.applyAppearance();
        ctx.refresh({ focus: '#appearance-ui-reset' });
        ctx.notify('仅界面字体与字号已恢复默认（当前演示）。', 'success');
      });
    }
  };
})();
