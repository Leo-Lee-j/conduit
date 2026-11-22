(() => {
  'use strict';
  const paths = {
    palette:'<circle cx="13.5" cy="6.5" r=".5"/><circle cx="17.5" cy="10.5" r=".5"/><circle cx="8.5" cy="7.5" r=".5"/><circle cx="6.5" cy="12.5" r=".5"/><path d="M12 3a9 9 0 1 0 0 18h1a2 2 0 0 0 1.2-3.6 1.3 1.3 0 0 1 .8-2.4h1a5 5 0 0 0 5-5c0-4-4-7-9-7Z"/>',
    keyboard:'<rect x="2" y="5" width="20" height="14" rx="2"/><path d="M6 9h.01M10 9h.01M14 9h.01M18 9h.01M6 12h.01M10 12h.01M14 12h.01M18 12h.01M7 15h10"/>',
    settings:'<path d="m12 3 2 3 3-.4.7 3 2.3 2-2.3 2-.7 3-3-.4-2 3-2-3-3 .4-.7-3L4 10.6l2.3-2 .7-3 3 .4Z"/><circle cx="12" cy="11" r="3"/>',
    terminal:'<rect x="3" y="4" width="18" height="16" rx="2"/><path d="m7 8 4 4-4 4m6 0h4"/>',
    server:'<rect x="3" y="3" width="18" height="7" rx="2"/><rect x="3" y="14" width="18" height="7" rx="2"/><path d="M7 6.5h.01M7 17.5h.01M11 6.5h6M11 17.5h6"/>',
    message:'<path d="M21 11a8 8 0 0 1-8 8H8l-5 3V6a3 3 0 0 1 3-3h7a8 8 0 0 1 8 8Z"/><path d="M7 8h10M7 12h6"/>',
    git:'<circle cx="6" cy="5" r="2"/><circle cx="6" cy="19" r="2"/><circle cx="18" cy="6" r="2"/><path d="M6 7v10m12-9v3a4 4 0 0 1-4 4H6"/>',
    blocks:'<rect x="3" y="3" width="7" height="7" rx="1"/><rect x="14" y="3" width="7" height="7" rx="1"/><rect x="3" y="14" width="7" height="7" rx="1"/><rect x="14" y="14" width="7" height="7" rx="1"/>',
    database:'<ellipse cx="12" cy="5" rx="8" ry="3"/><path d="M4 5v14c0 4 16 4 16 0V5M4 12c0 4 16 4 16 0"/>',
    chevron:'<path d="m9 5 7 7-7 7"/>',
    'arrow-left':'<path d="m12 5-7 7 7 7M5 12h14"/>',
    'arrow-right':'<path d="m12 5 7 7-7 7M19 12H5"/>',
    check:'<path d="m5 12 4 4L19 6"/>',
    alert:'<path d="m12 3 10 18H2L12 3Z"/><path d="M12 9v4m0 4h.01"/>',
    info:'<circle cx="12" cy="12" r="9"/><path d="M12 11v6m0-10h.01"/>',
    copy:'<rect x="8" y="8" width="12" height="13" rx="2"/><path d="M16 8V3H3v13h5"/>',
    plus:'<path d="M12 5v14M5 12h14"/>',
    x:'<path d="m6 6 12 12M18 6 6 18"/>',
    eye:'<path d="M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12Z"/><circle cx="12" cy="12" r="3"/>',
    refresh:'<path d="M20 7v5h-5M4 17v-5h5M5 7a8 8 0 0 1 14-1l1 6M4 12l1 6a8 8 0 0 0 14-1"/>',
    search:'<circle cx="10.5" cy="10.5" r="7"/><path d="m16 16 5 5"/>'
  };
  window.ConduitPrototype = {
    pages:{},
    escape(value){return String(value ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));},
    icon(name){return `<svg class="icon" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${paths[name] || paths.info}</svg>`;}
  };
})();
