(() => {
  'use strict';
  const P = window.ConduitPrototype;
  const $ = id => document.getElementById(id);
  const frame = $('settings-frame');
  const editor = $('editor-dialog');
  const discard = $('discard-dialog');
  const form = $('editor-form');
  const engines = ['Codex', 'Claude Code', 'OpenCode'];
  const navGroups = [
    ['工作台', [['通用','settings'],['外观与字体','palette','appearance'],['快捷键','keyboard']]],
    ['AI 与工作流', [['AI 引擎','terminal'],['模型与服务','server','services'],['会话设置','message','sessions'],['Git 工作流','git']]],
    ['扩展能力', [['Skills','blocks'],['MCP 服务','server'],['插件','blocks']]],
    ['应用维护', [['数据与更新','database']]]
  ];
  function initialState() {
    return {
      page:'appearance',engine:'Codex',readOnly:false,query:'',returned:false,scenario:'normal',
      appearance:{mode:'light',uiFont:'system',uiSize:16,chatFont:'system',chatSize:15,chatLineHeight:1.5,codeFont:'mono',codeSize:13,background:'solid'},
      session:{permission:'workspace',collapseReasoning:true,collapseQuestions:true,autoNaming:true,namingMode:'first',namingFollow:true},
      defaults:{'Codex':{serviceId:'codex-team',modelId:null},'Claude Code':{serviceId:null,modelId:null},'OpenCode':{serviceId:null,modelId:null}},
      services:[
        {id:'codex-team',name:'团队 API',engine:'Codex',source:'local',endpoint:'https://team.example.com/v1',defaultModel:'gpt-5-codex',models:[{id:'gpt-5-codex',name:'GPT-5 Codex'},{id:'gpt-5',name:'GPT-5'}],status:'untested',hasKey:true},
        {id:'codex-ccs',name:'日常账号',engine:'Codex',source:'ccs',endpoint:'https://gateway.example.com/v1',defaultModel:'gpt-5-codex',models:[{id:'gpt-5-codex',name:'GPT-5 Codex'}],status:'untested',hasKey:true},
        {id:'codex-sandbox',name:'测试服务',engine:'Codex',source:'local',endpoint:'https://sandbox.example.com/v1',defaultModel:'gpt-5',models:[{id:'gpt-5',name:'GPT-5'}],status:'untested',hasKey:false},
        {id:'claude-team',name:'团队 API',engine:'Claude Code',source:'local',endpoint:'https://claude.example.com',defaultModel:'claude-sonnet-4-5',models:[{id:'claude-sonnet-4-5',name:'Claude Sonnet 4.5'}],status:'untested',hasKey:true},
        {id:'claude-ccs',name:'日常账号',engine:'Claude Code',source:'ccs',endpoint:'https://claude-gateway.example.com',defaultModel:'claude-sonnet-4-5',models:[{id:'claude-sonnet-4-5',name:'Claude Sonnet 4.5'}],status:'untested',hasKey:true},
        {id:'opencode-team',name:'团队 API',engine:'OpenCode',source:'local',endpoint:'https://opencode.example.com/v1',defaultModel:'gpt-5-codex',models:[{id:'gpt-5-codex',name:'GPT-5 Codex'}],status:'untested',hasKey:true}
      ],
      serviceView:{selectedId:'codex-team',detailOpen:false},
      existingSessions:[
        {id:'session-navigation',name:'重构设置导航',engine:'Codex',serviceId:'codex-team',modelId:'gpt-5-codex'},
        {id:'session-paths',name:'修复长路径显示',engine:'Codex',serviceId:'codex-team',modelId:'gpt-5'},
        {id:'session-review',name:'检查工作区布局',engine:'Claude Code',serviceId:null,modelId:null}
      ]
    };
  }
  P.state = initialState();
  let contextVersion = 0;
  let editorMeta = null, editorDirty = false, editorReturn = null, editorInputFocus = null, afterDiscard = null;
  let layoutQueued = false;
  const nativeFamily = '"Segoe UI","Microsoft YaHei UI",sans-serif';
  function family(value) {
    const allowed = {'system':nativeFamily,'default':nativeFamily,'Segoe UI':'"Segoe UI","Microsoft YaHei UI",sans-serif','Microsoft YaHei UI':'"Microsoft YaHei UI","Segoe UI",sans-serif','mono':'"Cascadia Code",Consolas,monospace','Consolas':'Consolas,monospace','Cascadia Code':'"Cascadia Code",Consolas,monospace'};
    return allowed[value] || nativeFamily;
  }
  function notify(message, tone='success') {
    $('save-status').dataset.tone = tone;
    $('save-status').innerHTML = P.icon(tone === 'error' ? 'alert' : tone === 'info' ? 'info' : 'check') + '<span>' + P.escape(message) + '</span>';
  }
  function defaultStatus() {
    if (P.state.readOnly) notify('只读预览：设置不可更改，仍可浏览与搜索', 'info');
    else notify('偏好自动保存 · 仅更新本次预览', 'info');
  }
  function applyAppearance() {
    const a = P.state.appearance;
    a.uiSize = Math.max(12,Math.min(20,Math.round(Number(a.uiSize) || 16)));
    const dark = a.mode === 'dark' || (a.mode === 'system' && window.matchMedia('(prefers-color-scheme: dark)').matches);
    document.body.dataset.theme = dark ? 'dark' : 'light';
    document.documentElement.style.setProperty('--ui-font-size', a.uiSize+'px');
    document.documentElement.style.setProperty('--ui-font-family', family(a.uiFont));
    $('font-select').value = String(a.uiSize);
    $('theme-select').value = a.mode;
    $('readonly-toggle').checked = P.state.readOnly;
    $('engine-select').value = P.state.engine;
    $('editor-submit').disabled = P.state.readOnly;
    applyLayout();
  }
  function applyLayout() {
    if (!frame.isConnected) return;
    const priorFocus = document.activeElement;
    const scale = Math.max(1,P.state.appearance.uiSize/16);
    const width = Math.round(frame.getBoundingClientRect().width);
    const navWidth = Math.ceil(224*scale/4)*4;
    const navMode = width >= navWidth+440*scale+48 ? 'expanded' : 'compact';
    frame.style.setProperty('--nav-width',navWidth+'px');
    frame.style.setProperty('--nav-row',(Math.ceil(36*scale/4)*4)+'px');
    frame.style.setProperty('--control-height',(Math.ceil(40*scale/4)*4)+'px');
    frame.style.setProperty('--list-width',(Math.ceil(248*scale/4)*4)+'px');
    frame.dataset.navMode = navMode;
    const columnWidth = $('content-scroll').clientWidth;
    frame.dataset.contentMode = columnWidth-48 < 560*scale ? 'stacked' : 'columns';
    const padding = frame.dataset.contentMode === 'stacked' ? 40 : 48;
    const usable = columnWidth-padding;
    frame.dataset.serviceLayout = usable >= 248*scale+600*scale+24 ? 'split' : 'single';
    frame.dataset.appearanceLayout = usable >= 560*scale+320*scale+24 ? 'split' : 'stacked';
    frame.dataset.detailOpen = String(P.state.serviceView.detailOpen);
    if (!editor.open && !discard.open && priorFocus && frame.contains(priorFocus) && !priorFocus.getClientRects().length) {
      if (priorFocus.closest('#settings-navigation')) focusSelector('#category-select');
      else if (priorFocus.closest('[data-service-detail]')) focusSelector('#service-row-'+CSS.escape(P.state.serviceView.selectedId || '')) || focusSelector('#page-title');
      else if (priorFocus.closest('[data-service-list]')) focusSelector('#service-detail-heading') || focusSelector('#page-title');
      else focusSelector('#page-title');
    }
    $('layout-report').textContent = `实际主区 ${width}px · 字号 ${P.state.appearance.uiSize}px · ${navMode === 'expanded' ? '文字导航' : '分类选择'}${P.state.page === 'services' ? ' · '+(frame.dataset.serviceLayout === 'split' ? '列表＋详情' : '列表／详情单页') : ''}`;
  }
  function queueLayout() {
    if (layoutQueued) return;
    layoutQueued = true;
    requestAnimationFrame(() => { layoutQueued=false; applyLayout(); });
  }
  function setViewport(value) {
    frame.style.setProperty('--preview-width', value === 'auto' ? '1500px' : Number(value)+'px');
    queueLayout();
  }
  function matchingResults(query) {
    const words = query.trim().toLowerCase().split(/\s+/);
    if (!query.trim()) return [];
    return Object.entries(P.pages).flatMap(([page,def]) => (def.search || []).filter(item => words.every(word => `${def.title} ${item.title} ${item.keywords}`.toLowerCase().includes(word))).map(item => ({...item,page,pageTitle:def.title})));
  }
  function syncShell() {
    const s=P.state, searching=Boolean(s.query.trim());
    const def=P.pages[s.page];
    $('settings-navigation').innerHTML = navGroups.map(([title,items]) => '<div class="nav-group"><h2 class="nav-group-title">'+P.escape(title)+'</h2>'+items.map(([name,icon,page]) => `<button type="button" class="nav-item" ${page ? `id="nav-${page}" data-page="${page}" ${!searching && s.page===page ? 'aria-current="page"' : ''}` : 'disabled title="此页不在本次三页原型范围"'}>${P.icon(icon)}<span>${P.escape(name)}</span></button>`).join('')+'</div>').join('')+'<p class="nav-footnote">本轮覆盖三个页面<br>其余入口仅展示信息架构</p>';
    $('category-select').value=s.page;
    $('page-title').textContent=searching ? '搜索设置' : def?.title || '设置';
    $('page-summary').textContent=searching ? '搜索本原型中的三页设置，结果定位到分组' : def?.summary || '';
    $('engine-label').hidden=searching || s.page==='appearance';
    $('context-hint').textContent=searching ? '搜索结果 · 分组定位' : s.page==='appearance' ? '设备级偏好 · 即时预览' : '引擎是查看上下文 · 切换不会修改默认';
    if ($('settings-search').value!==s.query) $('settings-search').value=s.query;
    $('settings-header').hidden=s.returned;
    $('settings-body').hidden=s.returned;
    $('returned-view').hidden=!s.returned;
    $('save-scope').textContent=s.page==='services' ? '服务配置在编辑表单中保存 · 演示数据' : s.page==='sessions' ? '各分组作用范围不同 · 演示数据' : '本次预览 · 不写入应用';
  }
  function focusSelector(selector) {
    if (!selector) return false;
    try { const node=document.querySelector(selector); if(node && node.getClientRects().length && !node.disabled){node.focus({preventScroll:true});return true;} } catch { /* stale or invalid return target */ }
    return false;
  }
  function refresh(options={}) {
    const oldFocus=document.activeElement?.id;
    const scroll=$('content-scroll').scrollTop;
    syncShell();
    const root=document.createElement('div');
    root.className='prototype-page'; root.id='prototype-page-'+P.state.page;
    if(P.state.query.trim()) {
      const results=matchingResults(P.state.query);
      root.innerHTML='<p class="search-count">'+results.length+' 个匹配分组</p><div class="search-results">'+results.map((r,i)=>`<button class="search-result" type="button" data-search-index="${i}"><span>${P.escape(r.pageTitle)}</span><strong>${P.escape(r.title)}</strong><span>查看该分组，不更改设置</span></button>`).join('')+'</div>'+(results.length ? '' : '<div class="empty-state"><h2>没有匹配的设置</h2><p>试试“字体”“默认模型”或“API Key”，也可以通过分类浏览</p></div>');
      root.addEventListener('click',event=>{const node=event.target.closest('[data-search-index]');if(node){const r=results[Number(node.dataset.searchIndex)];navigate(r.page,r.id);}});
    } else {
      const def=P.pages[P.state.page];
      if(def) { root.innerHTML=def.render(ctx); }
      else root.innerHTML='<div class="empty-state"><h2>组件未加载</h2><p>请保留 HTML 与 settings-ui 文件夹的相对位置</p></div>';
    }
    $('page-content').replaceChildren(root);
    if(!P.state.query.trim()) P.pages[P.state.page]?.bind(root,ctx);
    $('content-scroll').scrollTop=scroll;
    applyAppearance();
    if(options.focus) focusSelector(options.focus);
    else if(oldFocus) focusSelector('#'+CSS.escape(oldFocus));
  }
  function setDefault(engine, serviceId, modelId) {
    if(P.state.readOnly){notify('只读预览不能更改默认配置','error');return false;}
    if(!engines.includes(engine)){notify('无法识别目标引擎','error');return false;}
    const service=serviceId==null ? null : P.state.services.find(item=>item.id===serviceId && item.engine===engine);
    if(serviceId!=null && !service){notify('服务不可用，请重新选择，不会自动改写默认','error');return false;}
    if(modelId!=null && (!service || !service.models.some(model=>model.id===modelId))){notify('模型不属于当前服务','error');return false;}
    P.state.defaults[engine]={serviceId:serviceId ?? null,modelId:modelId ?? null};
    notify('已更新 '+engine+' 的新会话默认，已有会话引用不变');
    return true;
  }
  function currentRoute(engine) {
    const route=P.state.defaults[engine] || {serviceId:null,modelId:null};
    if(!route.serviceId) return engine+' → 原生 CLI 配置 → 跟随 CLI 默认';
    const service=P.state.services.find(item=>item.id===route.serviceId && item.engine===engine);
    if(!service) return engine+' → 服务不可用，请重选';
    return engine+' → '+service.name+' → '+(route.modelId || service.defaultModel || '跟随服务默认');
  }
  function resetContextProbes() {
    contextVersion++;
    P.state.services.forEach(service=>{if(service.status==='testing')service.status='untested';});
  }
  function navigate(page,sectionId) {
    if(!P.pages[page]) return;
    guarded(() => {
      resetContextProbes(); P.state.page=page; P.state.query=''; P.state.returned=false;
      $('content-scroll').scrollTop=0; refresh();
      if(sectionId){const group=document.getElementById(sectionId);if(group){group.tabIndex=-1;group.scrollIntoView({block:'start',behavior:'instant'});group.focus({preventScroll:true});}}
      else focusSelector('#page-title');
      defaultStatus();
    });
  }
  function setEngine(engine) {
    if(!engines.includes(engine) || P.state.engine===engine) return;
    guarded(() => {
      resetContextProbes(); P.state.engine=engine;
      P.state.serviceView.selectedId=P.state.services.find(item=>item.engine===engine)?.id || null;
      P.state.serviceView.detailOpen=false;
      refresh({focus:'#engine-select'}); notify('正在查看 '+engine+'，未修改新会话默认','info');
    });
  }
  function returnToWorkbench() { guarded(()=>{resetContextProbes();P.state.returned=true;refresh();focusSelector('#reopen-settings');notify('已退出设置预览，配置数据仍只在内存中','info');}); }
  function guarded(action) { if(editor.open) requestEditorClose(action); else action(); }
  function openDialog(meta) {
    if(P.state.readOnly){notify('只读预览不能编辑服务','error');return false;}
    if(editor.open) return false;
    editorMeta=meta; editorDirty=false; afterDiscard=null;
    editorReturn=document.activeElement?.id ? '#'+CSS.escape(document.activeElement.id) : null;
    $('editor-title').textContent=meta.title;
    $('editor-description').textContent=meta.description || '只更新演示数据，不写入应用或凭据文件';
    $('editor-body').innerHTML=meta.content;
    $('editor-submit').textContent=meta.submitLabel || '保存';
    $('editor-submit').disabled=false;
    $('editor-error').hidden=true; $('editor-error').textContent='';
    editor.showModal(); meta.onMount?.($('editor-body'));
    editorInputFocus=$('editor-body').querySelector('input:not([type=hidden]),select,textarea');
    editorInputFocus?.focus(); return true;
  }
  function closeEditor(action=null) {
    if(discard.open) discard.close();
    editor.close(); $('editor-body').replaceChildren();
    editorMeta=null;editorDirty=false;afterDiscard=null;editorInputFocus=null;
    if(action) action(); else refresh({focus:editorReturn});
    editorReturn=null;
  }
  function requestEditorClose(action=null) {
    if(!editor.open){action?.();return;}
    if(!editorDirty){closeEditor(action);return;}
    afterDiscard=action;
    if(!discard.open){discard.showModal();$('continue-editing').focus();}
  }
  const ctx={
    get state(){return P.state;},escape:P.escape,icon:P.icon,isReadonly:()=>P.state.readOnly,
    refresh,notify,applyAppearance,setEngine,setDefault,currentRoute,navigate,openDialog,
    contextToken:()=>contextVersion,isContextCurrent:token=>token===contextVersion
  };
  P.ctx=ctx; P.refresh=refresh; P.layout=applyLayout; P.matchingResults=matchingResults;
  document.querySelectorAll('[data-icon]').forEach(node=>{node.innerHTML=P.icon(node.dataset.icon);});
  $('font-select').innerHTML=Array.from({length:9},(_,i)=>i+12).map(size=>'<option value="'+size+'">'+(size===16?'正常 · ':size===20?'大字号 · ':'')+size+'px</option>').join('');
  $('settings-navigation').addEventListener('click',event=>{const target=event.target.closest('[data-page]');if(target)navigate(target.dataset.page);});
  $('category-select').addEventListener('change',event=>navigate(event.target.value));
  $('engine-select').addEventListener('change',event=>setEngine(event.target.value));
  $('viewport-select').addEventListener('change',event=>setViewport(event.target.value));
  $('font-select').addEventListener('change',event=>{P.state.appearance.uiSize=Number(event.target.value);applyAppearance();refresh({focus:'#font-select'});});
  $('theme-select').addEventListener('change',event=>{P.state.appearance.mode=event.target.value;applyAppearance();refresh({focus:'#theme-select'});});
  $('scenario-select').addEventListener('change',event=>{P.state.scenario=event.target.value;notify('后续连接验证使用'+(event.target.value==='normal'?'模拟成功':'模拟失败')+'，不发起网络请求','info');});
  $('readonly-toggle').addEventListener('change',event=>{P.state.readOnly=event.target.checked;refresh({focus:'#readonly-toggle'});defaultStatus();});
  $('reset-demo').addEventListener('click',()=>guarded(()=>{resetContextProbes();P.state=initialState();$('scenario-select').value='normal';$('viewport-select').value='1200';setViewport('1200');refresh();defaultStatus();}));
  $('settings-back').addEventListener('click',returnToWorkbench);
  $('reopen-settings').addEventListener('click',()=>navigate(P.state.page));
  $('settings-search').addEventListener('input',event=>{if(event.isComposing)return;P.state.query=event.target.value;refresh({focus:'#settings-search'});});
  $('settings-search').addEventListener('compositionend',event=>{P.state.query=event.target.value;refresh({focus:'#settings-search'});});
  $('settings-search').addEventListener('keydown',event=>{if(event.key==='Enter'&&!event.isComposing){event.preventDefault();const first=matchingResults(P.state.query)[0];if(first)navigate(first.page,first.id);}});
  document.addEventListener('keydown',event=>{
    if(event.key!=='Escape' || editor.open || discard.open || event.defaultPrevented) return;
    event.preventDefault();
    if(P.state.query){P.state.query='';refresh({focus:'#settings-search'});}
    else if(P.state.page==='services' && P.state.serviceView.detailOpen && frame.dataset.serviceLayout==='single'){P.state.serviceView.detailOpen=false;refresh({focus:'#service-row-'+CSS.escape(P.state.serviceView.selectedId)});}
    else if(!P.state.returned) returnToWorkbench();
  });
  editor.addEventListener('cancel',event=>{event.preventDefault();requestEditorClose();});
  discard.addEventListener('cancel',event=>{event.preventDefault();discard.close();afterDiscard=null;editorInputFocus?.focus();});
  $('editor-body').addEventListener('input',()=>{editorDirty=true;});
  $('editor-body').addEventListener('change',()=>{editorDirty=true;});
  $('editor-body').addEventListener('focusin',event=>{if(event.target.matches('input,select,textarea'))editorInputFocus=event.target;});
  $('editor-cancel').addEventListener('click',()=>requestEditorClose());
  $('continue-editing').addEventListener('click',()=>{discard.close();afterDiscard=null;editorInputFocus?.focus();});
  $('discard-changes').addEventListener('click',()=>{const action=afterDiscard;closeEditor(action);notify('已放弃表单修改，未写入任何配置','info');});
  form.addEventListener('submit',event=>{
    event.preventDefault();
    if(P.state.readOnly){$('editor-error').hidden=false;$('editor-error').textContent='当前实例只读，不能保存';return;}
    if(!form.reportValidity())return;
    try {
      const error=editorMeta?.onSubmit?.(new FormData(form),$('editor-body'));
      if(typeof error==='string' && error.trim()){$('editor-error').hidden=false;$('editor-error').textContent=error;return;}
      closeEditor(); notify('服务表单已保存到本次演示，未写入应用');
    } catch { $('editor-error').hidden=false;$('editor-error').textContent='未能保存，请检查输入；当前草稿仍保留'; }
  });
  window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change',()=>{if(P.state.appearance.mode==='system')applyAppearance();});
  new ResizeObserver(queueLayout).observe(frame);
  window.addEventListener('resize',queueLayout);
  setViewport('1200'); refresh();defaultStatus();
})();
