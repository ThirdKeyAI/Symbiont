// Mermaid diagram rendering for Zensical
// Zensical wraps mermaid code in <pre class="mermaid"><code>...</code></pre>
// with HTML-encoded entities. We must clean up before mermaid can parse.
(function() {
  document.addEventListener('DOMContentLoaded', function() {
    // Step 1: Clean up mermaid blocks
    var blocks = document.querySelectorAll('pre.mermaid');
    blocks.forEach(function(pre) {
      var code = pre.querySelector('code');
      if (code) {
        // textContent auto-decodes HTML entities (&gt; -> >, etc.)
        var text = code.textContent;
        // Clear and set as direct text (no code wrapper)
        pre.innerHTML = '';
        pre.textContent = text;
      }
    });

    // Step 2: Load and initialize mermaid
    if (blocks.length > 0) {
      var script = document.createElement('script');
      script.src = 'https://cdn.jsdelivr.net/npm/mermaid@10/dist/mermaid.min.js';
      script.onload = function() {
        mermaid.initialize({
          startOnLoad: false,
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
        mermaid.run({ querySelector: '.mermaid' }).then(function() {
          // Add zoom cursors after successful render
          document.querySelectorAll('.mermaid svg').forEach(function(svg) {
            svg.style.cursor = 'zoom-in';
          });
        });
      };
      document.head.appendChild(script);
    }
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
