// Click-to-zoom for mermaid diagrams
document.addEventListener('DOMContentLoaded', function() {
  document.addEventListener('click', function(e) {
    var mermaid = e.target.closest('.mermaid');
    if (!mermaid) return;
    var svg = mermaid.querySelector('svg');
    if (!svg) return;

    var overlay = document.createElement('div');
    overlay.style.cssText = 'position:fixed;inset:0;z-index:9999;background:rgba(0,0,0,0.9);display:flex;align-items:center;justify-content:center;cursor:zoom-out;padding:2rem;';

    var clone = svg.cloneNode(true);
    clone.style.cssText = 'max-width:95vw;max-height:95vh;width:auto;height:auto;';
    clone.removeAttribute('width');
    clone.removeAttribute('height');

    overlay.appendChild(clone);
    overlay.addEventListener('click', function() { overlay.remove(); });
    document.addEventListener('keydown', function handler(e) {
      if (e.key === 'Escape') { overlay.remove(); document.removeEventListener('keydown', handler); }
    });
    document.body.appendChild(overlay);
  });

  // Add cursor hint to mermaid diagrams
  var style = document.createElement('style');
  style.textContent = '.mermaid svg { cursor: zoom-in; }';
  document.head.appendChild(style);
});
