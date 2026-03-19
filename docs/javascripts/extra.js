// Initialize mermaid with dark theme
document.addEventListener('DOMContentLoaded', function() {
  if (typeof mermaid !== 'undefined') {
    mermaid.initialize({
      startOnLoad: true,
      theme: 'dark',
      themeVariables: {
        primaryColor: '#14b8a6',
        primaryBorderColor: '#0d9488',
        primaryTextColor: '#e2e8f0',
        lineColor: '#94a3b8',
        secondaryColor: '#1e293b',
        tertiaryColor: '#0f172a'
      }
    });
    // Mermaid needs <pre class="mermaid"> without the inner <code> tag
    // Zensical renders ```mermaid as <pre class="mermaid"><code>...</code></pre>
    // Strip the <code> wrapper so mermaid can parse the content
    document.querySelectorAll('pre.mermaid code').forEach(function(code) {
      var pre = code.parentElement;
      pre.textContent = code.textContent;
    });
    // Re-run mermaid on cleaned elements
    mermaid.run();
  }
});

// Click-to-zoom for mermaid diagrams
// Uses event delegation so it works after mermaid renders asynchronously
document.addEventListener('click', function(e) {
  // Find closest mermaid container — could be pre.mermaid or div with svg
  var target = e.target.closest('pre.mermaid, .mermaid');
  if (!target) return;
  var svg = target.querySelector('svg') || (target.tagName === 'svg' ? target : null);
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
      clone.setAttribute('viewBox', bbox.x + ' ' + bbox.y + ' ' + bbox.width + ' ' + bbox.height);
    } catch(ex) {}
  }

  overlay.appendChild(clone);
  overlay.addEventListener('click', function() { overlay.remove(); });
  document.addEventListener('keydown', function handler(ev) {
    if (ev.key === 'Escape') { overlay.remove(); document.removeEventListener('keydown', handler); }
  });
  document.body.appendChild(overlay);
});

// Add zoom cursor to mermaid SVGs — retry since mermaid renders async
(function addCursors() {
  var svgs = document.querySelectorAll('pre.mermaid svg, .mermaid svg');
  svgs.forEach(function(svg) { svg.style.cursor = 'zoom-in'; });
  if (Date.now() - (window._mc || Date.now()) < 10000) setTimeout(addCursors, 500);
})();
window._mc = Date.now();
