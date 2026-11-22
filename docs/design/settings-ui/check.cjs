'use strict';
// Offline prototype QA. Run: node docs/design/settings-ui/check.cjs (reuse existing previews: --no-screenshots or --assert-only)
// Writes only previews/*.png and test-report.json beside this script. No server/install.
const fs = require('node:fs'), path = require('node:path'), os = require('node:os');
const vm = require('node:vm'), assert = require('node:assert/strict');
const { pathToFileURL, fileURLToPath } = require('node:url');
const DIR = __dirname, HTML = path.resolve(DIR, '..', 'settings-ui.html');
const SECRET = 'sk-demo-only-QA-sentinel-NOT-A-REAL-KEY';
const PAGES = ['appearance', 'sessions', 'services'];
const CAPTURE = !process.argv.some(arg => ['--assert-only', '--no-screenshots'].includes(arg)), PREVIEW_TARGET = 9;
const VARIANTS = [['normal', 1200, 16, 'light'], ['narrow', 640, 16, 'light'], ['large', 960, 20, 'dark']];
const report = { startedAt: new Date().toISOString(), scope: 'offline HTML demo only', mode: CAPTURE ? 'matrix-and-previews' : 'assertions-only',
  previewProvenance: CAPTURE ? 'captured this run' : 'existing iteration-2 images; not a visual retest', tests: [], previews: [], blockedRequests: [], limits: [
    'No native app/CLI is launched; existingSessions is checked as a complete immutable snapshot.',
    'This demo has engine/page context probes, not MCP/workspace/CLI or asynchronous global catalog controls.',
    'DPR 1.25/1.5 checks browser raster scaling, not native Windows DPI certification. Layout matrix uses UI sizes 16/20; UI 12 is a boundary interaction check. 200% zoom is not a release gate.',
    'ctx is used only for fixture/guard controls, modal font reflow and delivery of intercepted mock callbacks. Primary user flows use real DOM controls.',
  ] };
let browser;
const redact = value => String(value).split(SECRET).join('[REDACTED DEMO KEY]');
const canonical = value => path.resolve(value).toLowerCase();
const state = page => page.evaluate(() => JSON.parse(JSON.stringify(window.ConduitPrototype.ctx.state)));
const business = s => ({ appearance: s.appearance, session: s.session, defaults: s.defaults,
  services: s.services, existingSessions: s.existingSessions });
const settle = page => page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
function instrumentation() {
  const q = window.__settingsQA = { storage: [], calls: [], logs: [], jobs: [], hold: false, seq: 0 };
  for (const name of ['localStorage', 'sessionStorage']) Object.defineProperty(window, name, {
    configurable: true, get() { q.storage.push(name); throw new Error('Offline prototype must not access ' + name); },
  });
  for (const name of ['fetch', 'XMLHttpRequest', 'WebSocket', 'EventSource']) window[name] = function () {
    q.calls.push(name); throw new Error('Offline prototype must not invoke ' + name);
  };
  navigator.sendBeacon = () => { q.calls.push('sendBeacon'); return false; };
  for (const level of ['log', 'info', 'warn', 'error', 'debug']) {
    const original = console[level].bind(console);
    console[level] = (...args) => {
      q.logs.push(args.map(a => { try { return typeof a === 'string' ? a : JSON.stringify(a); } catch { return String(a); } }).join(' '));
      original(...args);
    };
  }
  const timeout = window.setTimeout.bind(window), clear = window.clearTimeout.bind(window);
  window.setTimeout = (fn, ms, ...args) => {
    if (q.hold && Number(ms) === 900 && typeof fn === 'function') {
      const job = { id: --q.seq, fn, args, cancelled: false }; q.jobs.push(job); return job.id;
    }
    return timeout(fn, ms, ...args);
  };
  window.clearTimeout = id => {
    const job = q.jobs.find(j => j.id === id); if (job) job.cancelled = true; else clear(id);
  };
  // Intentionally deliver even cancelled callbacks: cancellation alone is not stale-result safety.
  q.deliver = () => { const job = q.jobs.shift(); if (!job) throw new Error('No intercepted 900ms mock callback'); job.fn(...job.args); };
}
async function configure(page, width = 1200, size = 16, theme = 'light') {
  // auto + real viewport resizing also covers 800px, absent from the demo width select.
  await page.setViewportSize({ width: width + (width <= 640 ? 24 : 40), height: 1100 });
  await page.locator('#viewport-select').selectOption('auto');
  await page.locator('#font-select').selectOption(String(size));
  await page.locator('#theme-select').selectOption(theme);
  await settle(page);
  assert.equal(await page.locator('#settings-frame').evaluate(e => Math.round(e.getBoundingClientRect().width)), width, 'Requested main-area width');
  assert.equal(await page.locator('body').getAttribute('data-theme'), theme);
  assert.equal((await state(page)).appearance.uiSize, size);
}
async function navigate(page, id) {
  if (!(await page.locator('#settings-header').isVisible())) await page.locator('#reopen-settings').click();
  const nav = page.locator('#nav-' + id);
  if (await nav.isVisible()) await nav.click(); else await page.locator('#category-select').selectOption(id);
  await page.locator('#prototype-page-' + id).waitFor({ state: 'visible' });
  assert.equal((await state(page)).page, id); await settle(page);
}
async function browse(page, id) {
  if (await page.locator('#service-back').isVisible()) await page.locator('#service-back').click();
  await page.locator('#service-row-' + id).click();
  await page.locator('#service-detail-heading').waitFor({ state: 'visible' });
  assert.equal((await state(page)).serviceView.selectedId, id); assert.equal(await page.locator('#service-row-' + id).getAttribute('aria-current'), 'true'); await settle(page);
}
async function layout(page, collect = false) {
  const issues = await page.evaluate(() => {
    const errors = [], tolerance = 2;
    const visible = e => e.getClientRects().length && getComputedStyle(e).visibility !== 'hidden';
    const label = e => e.id ? '#' + e.id : e.tagName.toLowerCase() + '.' + [...e.classList].slice(0, 2).join('.');
    const doc = document.documentElement;
    if (doc.scrollWidth > doc.clientWidth + tolerance) errors.push({ element: 'document', width: doc.clientWidth, scrollWidth: doc.scrollWidth });
    const boxes = '#settings-frame,#settings-header,#context-toolbar,#settings-footer,#settings-navigation,#page-content,.prototype-page,.settings-section,.setting-row,.service-row,.service-layout,dialog[open],.dialog-body';
    for (const e of document.querySelectorAll(boxes)) if (visible(e)) {
      const overflow = getComputedStyle(e).overflowX;
      // A genuine inner scroll region is legal; hidden/clip must not conceal overflowing content.
      if (!['auto', 'scroll'].includes(overflow) && e.scrollWidth > e.clientWidth + tolerance)
        errors.push({ element: label(e), width: e.clientWidth, scrollWidth: e.scrollWidth, overflow });
    }
    for (const e of document.querySelectorAll('button,input:not([type=hidden]),select,textarea')) if (visible(e)) {
      const bounds = e.parentElement?.closest('.setting-control,.setting-row,.service-row,.section-action,.search-box,.context-toolbar,.dialog-body,.dialog-footer,.review-controls,.discard-actions,.settings-navigation');
      if (!bounds || !visible(bounds)) continue;
      let scrollable = false;
      for (let p = e.parentElement; p; p = p.parentElement) {
        if (['auto', 'scroll'].includes(getComputedStyle(p).overflowX) && p.scrollWidth > p.clientWidth + tolerance) { scrollable = true; break; }
        if (p === bounds) break;
      }
      const r = e.getBoundingClientRect(), b = bounds.getBoundingClientRect();
      if (!scrollable && (r.left < b.left - tolerance || r.right > b.right + tolerance))
        errors.push({ element: label(e), boundary: label(bounds), left: r.left, right: r.right, bounds: [b.left, b.right] });
      if (!['INPUT', 'TEXTAREA', 'SELECT'].includes(e.tagName) && e.scrollWidth > e.clientWidth + tolerance)
        errors.push({ element: label(e), width: e.clientWidth, scrollWidth: e.scrollWidth, reason: 'control content clipped' });
    }
    return errors;
  });
  if (collect) return issues;
  assert.deepEqual(issues, [], 'Horizontal page/container/control overflow: ' + JSON.stringify(issues));
}
async function audit(page, logs, errors) {
  const result = await page.evaluate(() => ({ state: JSON.stringify(window.ConduitPrototype.ctx.state),
    text: document.body.innerText, alerts: [...document.querySelectorAll('[role=alert]')].map(e => e.textContent),
    storage: window.__settingsQA.storage, calls: window.__settingsQA.calls, logs: window.__settingsQA.logs }));
  assert.equal(JSON.stringify(result).includes(SECRET), false, 'API key escaped its ephemeral input into state/UI/errors/logs');
  assert.deepEqual(result.storage, [], 'Storage access is forbidden, including reads');
  assert.deepEqual(result.calls, [], 'Network APIs are forbidden even if no request succeeds');
  assert.deepEqual(errors, [], 'Uncaught browser errors');
  assert.deepEqual(logs.filter(l => l.type === 'error'), [], 'Browser console errors');
}
async function run(context, name, fn) {
  const entry = { name, status: 'running' }, start = Date.now(), logs = [], errors = [];
  const requestStart = report.blockedRequests.length; let page;
  try {
    page = await context.newPage(); page.setDefaultTimeout(4000);
    page.on('console', m => logs.push({ type: m.type(), text: redact(m.text()) }));
    page.on('pageerror', e => errors.push(redact(e.message)));
    await page.goto(pathToFileURL(HTML).href, { waitUntil: 'load' });
    await page.waitForFunction(() => window.ConduitPrototype?.ctx && ['appearance', 'sessions', 'services'].every(id => typeof window.ConduitPrototype.pages[id]?.render === 'function'));
    const existing = (await state(page)).existingSessions;
    await fn(page);
    assert.deepEqual((await state(page)).existingSessions, existing, 'Complete existingSessions snapshot changed');
    await audit(page, logs, errors);
    assert.equal(report.blockedRequests.length, requestStart, 'Unexpected external or out-of-scope file request');
    entry.status = 'passed';
  } catch (error) { entry.status = 'failed'; entry.error = redact(error.stack || error); }
  finally {
    entry.durationMs = Date.now() - start; entry.browserErrors = errors;
    entry.consoleErrors = logs.filter(l => l.type === 'error');
    report.tests.push(entry); console.log(`${entry.status === 'passed' ? 'PASS' : 'FAIL'} ${name}`);
    if (entry.error) console.error(entry.error.split('\n').slice(0, 4).join('\n'));
    if (page) await page.close();
  }
}
const draft = page => page.locator('#editor-body').evaluate(root => [...root.querySelectorAll('input,textarea,select')].map(e => ({ id: e.id, name: e.name, type: e.type, value: e.value, checked: e.checked })));
const openAdd = async page => { await navigate(page, 'services'); await page.locator('#service-add').click(); await page.locator('#editor-dialog').waitFor({ state: 'visible' }); };
async function functional(context) {
  await run(context, 'Three-page navigation, grouped search, Enter and Esc', async p => {
    await configure(p);
    const sections = { appearance: 'appearance-theme', sessions: 'session-defaults', services: 'services-route' };
    for (const id of PAGES) { await navigate(p, id); assert.ok(await p.locator('#' + sections[id]).isVisible()); }
    for (const [query, id, group] of [['Consolas', 'appearance', 'appearance-code'], ['collapseReasoning', 'sessions', 'session-display'], ['cc-switch', 'services', 'services-browse']]) {
      await p.locator('#settings-search').fill(query);
      assert.ok(await p.locator('[data-search-index]').count(), 'Search returned no groups: ' + query);
      await p.locator('#settings-search').press('Enter');
      assert.equal((await state(p)).page, id);
      assert.equal(await p.evaluate(() => document.activeElement.id), group, 'Enter must focus the first result group');
      assert.equal(await p.locator('#settings-search').inputValue(), '');
    }
    await p.locator('#settings-search').fill('no-match-QA'); await p.keyboard.press('Escape');
    assert.equal((await state(p)).query, ''); assert.equal((await state(p)).returned, false);
    await p.keyboard.press('Escape'); assert.equal((await state(p)).returned, true);
    await p.locator('#reopen-settings').click(); assert.equal((await state(p)).returned, false);
  });
  await run(context, 'Compact category selector is keyboard-operable', async p => {
    await configure(p, 640); const select = p.locator('#category-select');
    assert.ok(await select.isVisible()); assert.equal(await select.isDisabled(), false);
    assert.ok(await select.evaluate(e => e.labels.length || e.getAttribute('aria-label') || e.getAttribute('aria-labelledby')), 'Category selector needs an accessible name');
    await select.focus(); await select.press('ArrowDown'); await p.keyboard.press('Enter');
    assert.equal((await state(p)).page, 'services');
    assert.equal(await p.evaluate(() => document.activeElement.id), 'page-title');
  });
  await run(context, 'Responsive focus fallback: navigation and hidden service detail', async p => {
    await configure(p); await navigate(p, 'sessions'); await p.locator('#nav-sessions').focus();
    let before = await state(p); await p.setViewportSize({ width: 664, height: 1100 }); await settle(p);
    assert.ok(await p.locator('#category-select').isVisible()); assert.equal(await p.evaluate(() => document.activeElement.id), 'category-select'); assert.deepEqual(await state(p), before);
    await configure(p); await navigate(p, 'services'); assert.equal((await state(p)).serviceView.detailOpen, false);
    await p.locator('#service-model').focus(); before = await state(p);
    await p.setViewportSize({ width: 664, height: 1100 }); await settle(p);
    assert.equal(await p.locator('[data-service-detail]').isVisible(), false); assert.ok(await p.locator('#service-row-codex-team').isVisible());
    assert.equal(await p.evaluate(() => document.activeElement.id), 'service-row-codex-team'); assert.deepEqual(await state(p), before);
  });
  await run(context, 'Appearance boundary keys and UI-only reset', async p => {
    await navigate(p, 'appearance'); await p.locator('#appearance-mode').selectOption('dark');
    const before = (await state(p)).appearance, input = p.locator('#appearance-ui-size');
    await input.focus(); await input.press('Home'); assert.equal((await state(p)).appearance.uiSize, 12);
    await input.press('End'); assert.equal((await state(p)).appearance.uiSize, 20);
    await p.locator('#appearance-ui-reset').click(); const after = (await state(p)).appearance;
    assert.equal(after.uiSize, 16); assert.equal(after.mode, 'dark');
    for (const key of ['chatFont', 'chatSize', 'chatLineHeight', 'codeFont', 'codeSize', 'background']) assert.deepEqual(after[key], before[key]);
  });
  await run(context, 'Service browsing/model preview does not default; explicit action writes defaults only', async p => {
    await navigate(p, 'services'); const initial = (await state(p)).defaults;
    await browse(p, 'codex-sandbox'); await p.locator('#service-model').selectOption('gpt-5');
    assert.deepEqual((await state(p)).defaults, initial);
    const before = await state(p); await p.locator('#service-set-default').click(); const after = await state(p);
    assert.deepEqual(after.defaults, { ...before.defaults, Codex: { serviceId: 'codex-sandbox', modelId: 'gpt-5' } }, 'Other engine defaults must be preserved');
    after.defaults = before.defaults; assert.deepEqual(after, before, 'Default action mutated state beyond defaults');
  });
  await run(context, 'Sessions service/model/native choices update defaults only', async p => {
    await navigate(p, 'sessions'); const before = await state(p);
    await p.locator('#sessions-service').selectOption('codex-ccs'); await p.locator('#sessions-model').selectOption('gpt-5-codex');
    let after = await state(p); assert.deepEqual(after.defaults, { ...before.defaults, Codex: { serviceId: 'codex-ccs', modelId: 'gpt-5-codex' } });
    after.defaults = before.defaults; assert.deepEqual(after, before);
    await p.locator('#sessions-service').selectOption('');
    assert.deepEqual((await state(p)).defaults.Codex, { serviceId: null, modelId: null });
    assert.ok(await p.locator('#sessions-model').isDisabled());
  });
  await run(context, 'Auto naming toggles preserve strategy; latest/first are real policy choices', async p => {
    await navigate(p, 'sessions'); const before = await state(p);
    await p.locator('#sessions-auto-naming').uncheck();
    assert.equal((await state(p)).session.autoNaming, false); assert.ok(await p.locator('#sessions-naming-mode').isDisabled());
    assert.equal((await state(p)).session.namingMode, before.session.namingMode);
    await p.locator('#sessions-auto-naming').check(); await p.locator('#sessions-naming-mode').selectOption('latest');
    assert.equal((await state(p)).session.namingMode, 'latest');
    await p.locator('#sessions-auto-naming').uncheck(); await p.locator('#sessions-auto-naming').check();
    assert.equal((await state(p)).session.namingMode, 'latest', 'Turning naming off must retain the real strategy');
    await p.locator('#sessions-naming-mode').selectOption('first'); const after = await state(p);
    assert.equal(after.session.namingMode, 'first'); assert.deepEqual(after.defaults, before.defaults); assert.deepEqual(after.services, before.services);
  });
  await run(context, 'Same service name across engines resolves by ID/source, not label', async p => {
    await navigate(p, 'services'); const before = await state(p);
    assert.equal(before.services.find(s => s.id === 'codex-team').name, before.services.find(s => s.id === 'claude-team').name);
    await browse(p, 'codex-team'); await p.locator('#engine-select').selectOption('Claude Code');
    assert.deepEqual((await state(p)).defaults, before.defaults); assert.equal(await p.locator('#service-row-codex-team').count(), 0);
    await browse(p, 'claude-team'); await p.locator('#service-model').selectOption('claude-sonnet-4-5');
    await p.locator('#service-set-default').click();
    assert.deepEqual((await state(p)).defaults, { ...before.defaults, 'Claude Code': { serviceId: 'claude-team', modelId: 'claude-sonnet-4-5' } });
    const saved = (await state(p)).defaults;
    assert.equal(await p.evaluate(() => window.ConduitPrototype.ctx.setDefault('Codex', 'claude-team', 'claude-sonnet-4-5')), false);
    assert.deepEqual((await state(p)).defaults, saved);
    await browse(p, 'claude-ccs'); assert.ok(await p.locator('#service-edit').isDisabled()); assert.ok(await p.locator('#service-delete').isDisabled());
  });
  await run(context, 'Unavailable default is preserved across browsing, without silent fallback', async p => {
    await p.evaluate(() => { const c = window.ConduitPrototype.ctx; c.state.defaults.Codex = { serviceId: 'missing-QA-service', modelId: 'missing-model' }; c.refresh(); });
    const defaults = (await state(p)).defaults;
    for (const id of ['sessions', 'services']) { await navigate(p, id); assert.ok(await p.locator('.inline-note.warning').count()); assert.deepEqual((await state(p)).defaults, defaults); }
  });
  await run(context, 'Independent editor validation/save; password masking and no secret indexing', async p => {
    await openAdd(p); const before = business(await state(p));
    await p.locator('#service-dialog-name').fill('QA isolated service');
    await p.locator('#service-dialog-models').fill('demo-model\nqa-model');
    await p.locator('#service-dialog-defaultModel').fill('not-in-directory');
    await p.locator('#editor-submit').click(); assert.ok(await p.locator('#editor-error').isVisible());
    assert.deepEqual(business(await state(p)), before);
    const key = p.locator('#service-dialog-apiKey'); assert.equal(await key.getAttribute('type'), 'password');
    await key.fill(SECRET); await audit(p, [], []);
    const reveal = p.locator('[data-action=service-reveal]'); await reveal.focus();
    await reveal.press('Space'); assert.equal(await key.getAttribute('type'), 'text'); assert.equal(await reveal.getAttribute('aria-pressed'), 'true');
    await reveal.press('Space'); assert.equal(await key.getAttribute('type'), 'password'); assert.equal(await reveal.getAttribute('aria-pressed'), 'false');
    await p.locator('#service-dialog-defaultModel').fill('qa-model'); await p.locator('#editor-submit').click();
    assert.equal(await p.locator('#editor-dialog').isVisible(), false); const after = await state(p);
    assert.equal(after.services.length, before.services.length + 1); assert.deepEqual(after.defaults, before.defaults);
    const created = after.services.find(s => s.name === 'QA isolated service'); assert.ok(created.hasKey); assert.equal(created.defaultModel, 'qa-model'); assert.equal(await p.locator('#service-dialog-apiKey').count(), 0);
    // A unique fragment tests indexing without injecting the full sentinel into state.query ourselves.
    await p.locator('#settings-search').fill('QA-sentinel'); assert.equal(await p.locator('[data-search-index]').count(), 0);
    await p.keyboard.press('Escape');
  });
  await run(context, 'Shared editor retains entity identity and routes; requires impact confirmation', async p => {
    await navigate(p, 'services'); await browse(p, 'codex-team'); const before = await state(p);
    assert.ok(await p.locator('#service-delete').isDisabled()); await p.locator('#service-edit').click();
    assert.ok(await p.locator('#service-dialog-impact').isVisible());
    await p.locator('#service-dialog-endpoint').fill('https://edited.example.com/v1'); await p.locator('#editor-submit').click();
    assert.ok(await p.locator('#editor-dialog').isVisible()); assert.deepEqual((await state(p)).services, before.services);
    await p.locator('#service-dialog-impact').check(); await p.locator('#editor-submit').click(); const after = await state(p);
    assert.equal(after.services.find(s => s.id === 'codex-team').endpoint, 'https://edited.example.com/v1');
    assert.equal(after.services.length, before.services.length); assert.deepEqual(after.defaults, before.defaults);
  });
  await run(context, 'Dirty Esc: continue retains input/focus; explicit discard writes nothing', async p => {
    await openAdd(p); const before = business(await state(p)), name = p.locator('#service-dialog-name');
    await name.fill('QA unsaved draft'); await name.focus(); await p.keyboard.press('Escape');
    assert.ok(await p.locator('#discard-dialog').isVisible()); await p.locator('#continue-editing').click();
    assert.equal(await name.inputValue(), 'QA unsaved draft'); assert.equal(await p.evaluate(() => document.activeElement.id), 'service-dialog-name');
    await p.keyboard.press('Escape'); await p.locator('#discard-changes').click();
    assert.equal(await p.locator('#editor-dialog').isVisible(), false); assert.deepEqual(business(await state(p)), before);
  });
  await run(context, 'Window/font reflow preserves modal draft and focused input', async p => {
    await configure(p); await openAdd(p); await p.locator('#service-dialog-name').fill('QA reflow draft');
    await p.locator('#service-dialog-name').focus(); const before = await draft(p), node = await p.locator('#service-dialog-name').elementHandle();
    await p.setViewportSize({ width: 504, height: 900 }); await settle(p);
    assert.deepEqual(await draft(p), before); assert.equal(await p.evaluate(() => document.activeElement.id), 'service-dialog-name');
    // Native modal makes outside review controls inert; use the contracted non-rebuilding font API.
    await p.evaluate(() => { const c = window.ConduitPrototype.ctx; c.state.appearance.uiSize = 20; c.applyAppearance(); }); await settle(p);
    assert.deepEqual(await draft(p), before); assert.equal(await p.evaluate(() => document.activeElement.id), 'service-dialog-name');
    assert.ok(await node.evaluate(e => e.isConnected && e === document.getElementById('service-dialog-name')), 'Font/viewport reflow rebuilt the form'); await node.dispose(); await layout(p);
  });
  await run(context, 'Dirty cross-engine/page requests cannot silently discard or retarget edits', async p => {
    await openAdd(p); await p.locator('#service-dialog-name').fill('QA pending context'); const before = business(await state(p));
    // Simulates an app-level context request: background DOM is deliberately inert under a modal.
    await p.evaluate(() => window.ConduitPrototype.ctx.setEngine('Claude Code')); await p.locator('#continue-editing').click();
    assert.equal((await state(p)).engine, 'Codex'); assert.equal(await p.locator('#service-dialog-name').inputValue(), 'QA pending context');
    await p.evaluate(() => window.ConduitPrototype.ctx.navigate('sessions')); await p.locator('#discard-changes').click();
    assert.equal((await state(p)).page, 'sessions'); assert.deepEqual(business(await state(p)), before);
  });
  await run(context, 'Read-only keeps navigation/browsing enabled and blocks every demo write control', async p => {
    await configure(p, 640, 20); await p.locator('#readonly-toggle').check(); const before = business(await state(p));
    for (const id of PAGES) {
      await navigate(p, id);
      let writers;
      if (id === 'appearance') writers = '[data-appearance-key],#appearance-ui-reset';
      else if (id === 'sessions') { await p.locator('#engine-select').selectOption('Claude Code'); writers = '#sessions-service,#sessions-model,#sessions-permission,#sessions-collapse-reasoning,#sessions-collapse-questions,#sessions-auto-naming,#sessions-naming-mode'; }
      else { await p.locator('#engine-select').selectOption('Codex'); await browse(p, 'codex-sandbox'); writers = '#service-add,#service-test,#service-edit,#service-delete,#service-set-default,#service-native-default'; await p.locator('#service-model').selectOption('gpt-5'); }
      for (const e of await p.locator(writers).all()) assert.ok(await e.isDisabled(), 'Read-only write control enabled: ' + await e.getAttribute('id'));
      assert.deepEqual(business(await state(p)), before); await layout(p);
    }
    assert.equal(await p.evaluate(() => window.ConduitPrototype.ctx.setDefault('Codex', null, null)), false);
    assert.deepEqual(business(await state(p)), before);
  });
  await run(context, 'Read-only transition rejects an already open editor submission', async p => {
    await openAdd(p); await p.locator('#service-dialog-name').fill('QA read-only draft'); const before = business(await state(p));
    // The modal makes the outside mode toggle inert; exercise the central submit guard via native requestSubmit.
    await p.evaluate(() => { const c = window.ConduitPrototype.ctx; c.state.readOnly = true; c.applyAppearance(); });
    assert.ok(await p.locator('#editor-submit').isDisabled());
    await p.locator('#editor-form').evaluate(form => form.requestSubmit());
    assert.ok(await p.locator('#editor-error').isVisible()); assert.ok(await p.locator('#editor-dialog').isVisible());
    assert.deepEqual(business(await state(p)), before); assert.equal(await p.locator('#service-dialog-name').inputValue(), 'QA read-only draft');
  });
  await run(context, 'Mock verification failure is visible, isolated and offline', async p => {
    await navigate(p, 'services'); await browse(p, 'codex-sandbox'); const defaults = (await state(p)).defaults;
    await p.locator('#scenario-select').selectOption('verification-failure'); await p.evaluate(() => { window.__settingsQA.hold = true; });
    await p.locator('#service-test').click(); assert.equal((await state(p)).services.find(s => s.id === 'codex-sandbox').status, 'testing');
    await p.evaluate(() => window.__settingsQA.deliver());
    assert.equal((await state(p)).services.find(s => s.id === 'codex-sandbox').status, 'failed');
    assert.ok(await p.locator('[data-service-detail] [data-status=failed]').isVisible()); assert.deepEqual((await state(p)).defaults, defaults);
  });
  for (const change of ['engine', 'engine-roundtrip', 'page']) for (const outcome of ['normal', 'verification-failure']) await run(context, `Late ${outcome} callback after ${change} change is ignored`, async p => {
    await navigate(p, 'services'); await browse(p, 'codex-sandbox'); await p.locator('#scenario-select').selectOption(outcome);
    await p.evaluate(() => { window.__settingsQA.hold = true; }); await p.locator('#service-test').click();
    assert.equal(await p.evaluate(() => window.__settingsQA.jobs.length), 1, 'Mock timer interception must be active');
    let current = 'codex-sandbox';
    if (change.startsWith('engine')) {
      await p.locator('#engine-select').selectOption('Claude Code'); current = 'claude-team';
      if (change === 'engine-roundtrip') { await p.locator('#engine-select').selectOption('Codex'); current = 'codex-sandbox'; }
    }
    else { await navigate(p, 'sessions'); await navigate(p, 'services'); }
    await browse(p, current); await p.locator('#scenario-select').selectOption('normal'); await p.locator('#service-test').click();
    const before = await state(p); await p.evaluate(() => window.__settingsQA.deliver());
    assert.deepEqual(await state(p), before, 'Delivered stale result changed current or prior state');
    await p.evaluate(() => window.__settingsQA.deliver());
    assert.equal((await state(p)).services.find(s => s.id === current).status, 'passed'); assert.deepEqual((await state(p)).defaults, before.defaults);
  });
}
async function main() {
  for (const file of ['core.js', 'appearance.js', 'sessions.js', 'services.js', 'app.js']) new vm.Script(fs.readFileSync(path.join(DIR, file), 'utf8'), { filename: file });
  assert.ok(fs.existsSync(HTML), 'Missing settings-ui.html');
  const tools = process.env.PLAYWRIGHT_MODULE || process.env.CONDUIT_PLAYWRIGHT_MODULE || path.join(os.tmpdir(), 'conduit-settings-prototype-tools/node_modules/playwright');
  const { chromium } = require(tools);
  const candidates = [process.env.PROTOTYPE_BROWSER, process.env.CONDUIT_BROWSER_PATH, process.env.CHROME_PATH, process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH,
    'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe', 'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe', 'C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe'];
  const executablePath = candidates.find(p => p && fs.existsSync(p)); assert.ok(executablePath, 'Chrome/Edge not found; set PROTOTYPE_BROWSER');
  report.executablePath = executablePath;
  browser = await chromium.launch({ executablePath, headless: true, args: ['--disable-background-networking', '--disable-component-update', '--disable-sync', '--no-first-run'] });
  report.browserVersion = browser.version();
  const allowed = new Set([HTML, ...['core.js', 'app.js', 'styles.css', ...PAGES.map(id => id + '.js')].map(f => path.join(DIR, f)), path.resolve(DIR, '../../../assets/app-icons/32x32.png')].map(canonical));
  const contexts = [];
  for (const deviceScaleFactor of [1.25, 1.5]) {
    const context = await browser.newContext({ viewport: { width: 1240, height: 1100 }, deviceScaleFactor, locale: 'zh-CN', offline: true, serviceWorkers: 'block', acceptDownloads: false, reducedMotion: 'reduce' });
    await context.addInitScript(instrumentation);
    await context.route('**/*', async route => {
      const url = route.request().url(); let permitted = false;
      try { permitted = url.startsWith('file:') && allowed.has(canonical(fileURLToPath(url))); } catch { /* deny malformed URLs */ }
      if (permitted) await route.continue(); else { report.blockedRequests.push(redact(url)); await route.abort('blockedbyclient'); }
    });
    contexts.push({ context, deviceScaleFactor });
  }
  await functional(contexts[0].context);
  for (const { context, deviceScaleFactor: dpr } of contexts) for (const width of [480, 640, 800, 960, 1200]) for (const size of [16, 20]) for (const theme of ['light', 'dark']) for (const id of PAGES)
    await run(context, `Layout ${id} ${width}px font${size} ${theme} DPR${dpr}`, async p => {
      await configure(p, width, size, theme); await navigate(p, id); const issues = await layout(p, true);
      if (id === 'services') { await browse(p, 'codex-team'); issues.push(...await layout(p, true)); }
      assert.deepEqual(issues, [], 'Horizontal overflow in list/detail: ' + JSON.stringify(issues));
    });
  if (CAPTURE) {
  fs.mkdirSync(path.join(DIR, 'previews'), { recursive: true });
  // One bounded reference-render batch; screenshots always use fresh state and no open editor/key.
  for (const id of PAGES) for (const [variant, width, size, theme] of VARIANTS)
    await run(contexts[0].context, `Preview ${id} ${variant}`, async p => {
      await configure(p, width, size, theme); await navigate(p, id); if (id === 'services') await browse(p, 'codex-team');
      assert.equal(await p.locator('#editor-dialog').isVisible(), false); assert.equal(JSON.stringify(await state(p)).includes(SECRET), false);
      const relative = `previews/${id}-${variant}-${width}.png`;
      await p.locator('#settings-frame').screenshot({ path: path.join(DIR, relative), animations: 'disabled' });
      report.previews.push({ path: relative, page: id, width, uiSize: size, theme, deviceScaleFactor: 1.25, capturedThisRun: true });
    });
  } else for (const id of PAGES) for (const [variant, width, uiSize, theme] of VARIANTS) {
    const relative = `previews/${id}-${variant}-${width}.png`, saved = fs.statSync(path.join(DIR, relative));
    assert.ok(saved.size > 8, 'Missing/empty existing preview: ' + relative);
    report.previews.push({ path: relative, page: id, width, uiSize, theme, deviceScaleFactor: 1.25, capturedThisRun: false, lastModified: saved.mtime.toISOString() });
  }
  assert.equal(report.previews.length, PREVIEW_TARGET, 'Reference preview count does not match run mode');
  for (const { context } of contexts) await context.close();
}
main().catch(error => { report.fatal = redact(error.stack || error); console.error(report.fatal); }).finally(async () => {
  try { if (browser) await browser.close(); } catch (error) { report.fatal = report.fatal || redact(error.stack || error); }
  report.finishedAt = new Date().toISOString();
  report.summary = { passed: report.tests.filter(t => t.status === 'passed').length, failed: report.tests.filter(t => t.status === 'failed').length, previews: report.previews.length, captured: report.previews.filter(p => p.capturedThisRun).length, reused: report.previews.filter(p => !p.capturedThisRun).length };
  report.status = report.fatal || report.summary.failed || report.previews.length !== PREVIEW_TARGET ? 'failed' : 'passed';
  try { fs.writeFileSync(path.join(DIR, 'test-report.json'), JSON.stringify(report, null, 2) + '\n', 'utf8'); }
  catch (error) { console.error('Cannot write test-report.json:', error.message); report.status = 'failed'; }
  console.log(`${report.status.toUpperCase()}: ${report.summary.passed} passed / ${report.summary.failed} failed; ${report.summary.previews}/${PREVIEW_TARGET} previews (${report.summary.captured} captured, ${report.summary.reused} reused)`);
  process.exitCode = report.status === 'passed' ? 0 : 1;
});
