// === Language selector fix ===
// Rewrite language links to include current page path
document.addEventListener('DOMContentLoaded', function() {
  var path = location.pathname;
  var langPrefixes = ['/zh-cn/', '/es/', '/pt/', '/ja/', '/de/'];
  var pagePath = path;
  langPrefixes.forEach(function(prefix) {
    if (path.startsWith(prefix)) pagePath = path.substring(prefix.length - 1);
  });
  if (pagePath === '') pagePath = '/';

  document.querySelectorAll('a.md-select__link[hreflang]').forEach(function(a) {
    var lang = a.getAttribute('hreflang');
    if (lang === 'en') {
      a.setAttribute('href', pagePath);
    } else {
      var langDir = { zh: 'zh-cn', es: 'es', pt: 'pt', ja: 'ja', de: 'de' }[lang] || lang;
      a.setAttribute('href', '/' + langDir + pagePath);
    }
  });
});

// === Click-to-zoom for mermaid diagrams ===
document.addEventListener('click', function(e) {
  var target = e.target.closest('.mermaid');
  if (!target) return;
  var svg = target.querySelector('svg');
  if (!svg) return;

  var overlay = document.createElement('div');
  overlay.style.cssText = 'position:fixed;inset:0;z-index:9999;background:rgba(0,0,0,0.92);display:flex;align-items:center;justify-content:center;cursor:zoom-out;padding:2rem;';

  var clone = svg.cloneNode(true);
  clone.style.cssText = 'max-width:95vw;max-height:95vh;width:auto;height:auto;';
  clone.removeAttribute('width');
  clone.removeAttribute('height');
  if (!clone.getAttribute('viewBox') && svg.getBBox) {
    try {
      var bbox = svg.getBBox();
      clone.setAttribute('viewBox', bbox.x+' '+bbox.y+' '+bbox.width+' '+bbox.height);
    } catch(ex) {}
  }

  overlay.appendChild(clone);
  overlay.addEventListener('click', function() { overlay.remove(); });
  document.addEventListener('keydown', function handler(ev) {
    if (ev.key === 'Escape') { overlay.remove(); document.removeEventListener('keydown', handler); }
  });
  document.body.appendChild(overlay);
});

// Add zoom cursor to mermaid SVGs after they render
(function waitForMermaid() {
  var svgs = document.querySelectorAll('.mermaid svg');
  if (svgs.length > 0) {
    svgs.forEach(function(svg) { svg.style.cursor = 'zoom-in'; });
  }
  if (Date.now() - (window._mc || Date.now()) < 15000) setTimeout(waitForMermaid, 1000);
})();
window._mc = Date.now();
