// Step 1: Strip <code> wrappers from mermaid blocks IMMEDIATELY
// (before any DOMContentLoaded, as early as possible)
(function() {
  function stripCodeWrappers() {
    document.querySelectorAll('pre.mermaid code').forEach(function(code) {
      var pre = code.parentElement;
      var text = code.textContent;
      pre.textContent = text;
    });
  }

  // Try now (if DOM is ready)
  if (document.readyState !== 'loading') {
    stripCodeWrappers();
  }

  // Also on DOMContentLoaded (belt and suspenders)
  document.addEventListener('DOMContentLoaded', function() {
    stripCodeWrappers();

    // Step 2: Load mermaid dynamically AFTER cleanup
    var script = document.createElement('script');
    script.src = 'https://cdn.jsdelivr.net/npm/mermaid@10/dist/mermaid.min.js';
    script.onload = function() {
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
      mermaid.run({ querySelector: '.mermaid' });

      // Add zoom cursors after render
      setTimeout(function() {
        document.querySelectorAll('.mermaid svg').forEach(function(svg) {
          svg.style.cursor = 'zoom-in';
        });
      }, 1000);
    };
    document.body.appendChild(script);
  });
})();

// Click-to-zoom for mermaid diagrams
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
