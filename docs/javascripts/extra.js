// Click-to-zoom for mermaid diagrams
// Uses event delegation so it works even after mermaid renders asynchronously
document.addEventListener('click', function(e) {
  var mermaid = e.target.closest('.mermaid');
  if (!mermaid) return;
  var svg = mermaid.querySelector('svg');
  if (!svg) return;

  var overlay = document.createElement('div');
  overlay.style.cssText = 'position:fixed;inset:0;z-index:9999;background:rgba(0,0,0,0.92);display:flex;align-items:center;justify-content:center;cursor:zoom-out;padding:2rem;';

  var clone = svg.cloneNode(true);
  clone.style.cssText = 'max-width:95vw;max-height:95vh;width:auto;height:auto;';
  clone.removeAttribute('width');
  clone.removeAttribute('height');
  // Ensure viewBox is set so it scales properly
  if (!clone.getAttribute('viewBox') && svg.getBBox) {
    try {
      var bbox = svg.getBBox();
      clone.setAttribute('viewBox', bbox.x + ' ' + bbox.y + ' ' + bbox.width + ' ' + bbox.height);
    } catch(e) {}
  }

  overlay.appendChild(clone);
  overlay.addEventListener('click', function() { overlay.remove(); });
  document.addEventListener('keydown', function handler(ev) {
    if (ev.key === 'Escape') { overlay.remove(); document.removeEventListener('keydown', handler); }
  });
  document.body.appendChild(overlay);
});

// Add zoom cursor to mermaid diagrams — keep retrying since mermaid renders async
(function addCursors() {
  var diagrams = document.querySelectorAll('.mermaid svg');
  if (diagrams.length > 0) {
    diagrams.forEach(function(svg) { svg.style.cursor = 'zoom-in'; });
  }
  // Keep checking for new mermaid renders for 10 seconds
  if (document.readyState !== 'complete' || Date.now() - window._mermaidCursorStart < 10000) {
    setTimeout(addCursors, 500);
  }
})();
window._mermaidCursorStart = Date.now();
