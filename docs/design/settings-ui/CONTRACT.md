# Settings prototype page contract

This is an offline HTML prototype, not the native application. No fetch, external assets, storage, real CLI, credential persistence, or changes to src/. Scripts are plain deferred browser scripts, not ESM, so opening settings-ui.html directly works.

## Ownership

- Coordinator: ../settings-ui.html, core.js, app.js, styles.css, prototype checks and documentation.
- Appearance worker: appearance.js ONLY.
- Product worker: sessions.js ONLY.
- Services worker: services.js ONLY.

Register `window.ConduitPrototype.pages.PAGE = { title, summary, search: [{id,title,keywords}], render(ctx), bind(root,ctx) }`. PAGE is appearance, sessions, or services. render returns escaped HTML; bind runs on a newly rendered page root. Never touch shell DOM. Use IIFE + strict mode. Scripts are loaded after core.js and before app.js.

## Shared API (ctx)

- state: mutable in-memory demo state described below. Never mutate existingSessions.
- escape(value), icon(name): safe text and inline SVG. Icon names: palette, keyboard, settings, terminal, server, message, git, blocks, database, chevron, arrow-left, arrow-right, check, alert, info, copy, plus, x, eye, refresh, search.
- isReadonly(): state.readOnly. Controls that write settings must be disabled; browsing, navigation and engine choice remain available.
- refresh({focus: CSS_SELECTOR} = {}): refresh current page, restore focused ID where possible, apply theme/layout. Do not call during typing in a dialog.
- notify(message, tone='success'): prototype-only status. tone may be success, info, error.
- applyAppearance(): update theme, font and responsive geometry without rebuilding a page or dialog.
- setEngine(engine): update inspection context and invalidate contextual mock probes.
- setDefault(engine, serviceId, modelId): validates service engine/ID, checks readonly, writes defaults ONLY; returns bool, does not refresh. null serviceId means native CLI, null modelId means follow service default.
- currentRoute(engine): escaped by caller, readable engine/service/model summary.
- navigate(page, sectionId): switch prototype page, optional group scroll/focus (NOT field-level search).
- contextToken(), isContextCurrent(token): contextual mock verification safety.
- openDialog({ title, description, content, submitLabel='保存', onMount(body), onSubmit(formData,body) }): opens a real modal dialog with central dirty-input and Esc/leave confirmation. content contains form fields ONLY, no nested form or dialog/action footer. onSubmit returns a non-empty error string to remain open, otherwise success. Coordinator checks readonly BEFORE calling it, closes on success and refreshes page. Never save real secret text to state; use hasKey boolean. onMount may bind reveal control. Central cancel preserves draft unless explicitly discarded.

## In-memory state

state.engine: 'Codex' | 'Claude Code' | 'OpenCode'.
state.appearance: {mode:'light'|'dark'|'system',uiFont:'system',uiSize:16,chatFont:'system',chatSize:15,chatLineHeight:1.5,codeFont:'mono',codeSize:13,background:'solid'}.
state.session: {permission:'workspace'|'readonly'|'full',collapseReasoning:true,collapseQuestions:true,autoNaming:true,namingMode:'first'|'latest',namingFollow:true}.
state.defaults: map per engine -> {serviceId: string|null, modelId:string|null}.
state.services: array of {id,name,engine,source:'local'|'ccs',endpoint,defaultModel,models:[{id,name}],status:'untested'|'testing'|'passed'|'failed',hasKey:boolean}. Coordinator provides demo local and cc-switch services, names and endpoints.
state.serviceView: {selectedId:string|null,detailOpen:false}; detailOpen means user selected detail on narrow layout.
state.existingSessions: immutable sample array of {id,name,engine,serviceId,modelId}. Defaults must never rewrite it.
state.scenario: 'normal'|'verification-failure' (readonly is separate).

## Shared HTML/CSS vocabulary

settings-section, section-heading, section-title, section-description, section-action; setting-row, setting-copy, setting-label, setting-description, setting-control, scope-note; btn, btn-primary, btn-quiet, btn-danger; switch (native checkbox role=switch in label, use span.switch-track); control, control-small; inline-note, inline-note.warning; preview-card, preview-header, preview-body; mono, muted, status, default-label; empty-state.

Service responsive CSS is coordinator-owned: .service-layout contains .service-list[data-service-list] and .service-detail[data-service-detail]. Frame data-service-layout is 'split' or 'single'; data-detail-open is 'true' or 'false'. Never hide panels in JS based on a hard pixel breakpoint. In single mode the list or detail is displayed according to detailOpen; in split mode both display. Make back-to-list `[data-action="service-back"]` with class `service-back` (hidden only in split). Use service row buttons with id `service-row-ID` and data-service-id. Lists preserve browse selection independently from defaults. Long paths/IDs wrap; action buttons wrap or stack. Existing native CLI model catalog remains a separate note/link, never masquerades as a saved service.

## Scope/copy requirements

- All data and verification are explicitly demo-only. Use example.com endpoints, fake model catalogs and a clearly labelled mock-test action. No real network requests.
- Appearance: user theme/fonts respected, show a useful live conversation/code preview; UI/chat/code independent. UI size 12–20. UI reset affects UI font/size only.
- Sessions: separate four sections: new session defaults (per engine), default permission, display, automatic naming. Display is immediate, naming may affect old sessions' later rounds; do not label all sections only-new-session. Native CLI needs no added service. Default setting never changes existingSessions.
- Services: browse, default and verification are separate states; only explicit default action mutates defaults. Reuse separate editing modal; warn before editing shared service referenced by existingSessions. CCS source cannot be edited/deleted as local. Local delete blocked when referenced by sessions or defaults. Mock tests bind contextToken and ignore late results. Do not store typed API key text.
- Native labelled controls and visible keyboard focus; no emoji, decoration, external fonts, gradients, fake production claims, or per-setting cards.