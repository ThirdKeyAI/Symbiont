// === 1. Hide mermaid source to prevent flash ===
document.head.insertAdjacentHTML('beforeend',
  '<style>pre.mermaid{visibility:hidden;height:0;overflow:hidden}</style>');

// === 2. Mermaid rendering ===
document.addEventListener('DOMContentLoaded', function() {
  var pres = document.querySelectorAll('pre.mermaid');
  if (pres.length === 0) return;

  // Collect diagram definitions
  var diagrams = [];
  pres.forEach(function(pre, i) {
    var code = pre.querySelector('code');
    var text = code ? code.textContent : pre.textContent;
    diagrams.push({ element: pre, text: text, id: 'mmd-' + Date.now() + '-' + i });
  });

  // Load mermaid
  var s = document.createElement('script');
  s.src = 'https://cdn.jsdelivr.net/npm/mermaid@10.9.3/dist/mermaid.min.js';
  s.onload = function() {
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

    // Render each diagram individually using render() API
    diagrams.forEach(function(d) {
      mermaid.render(d.id, d.text).then(function(result) {
        var div = document.createElement('div');
        div.className = 'mermaid';
        div.innerHTML = result.svg;
        div.querySelector('svg').style.cursor = 'zoom-in';
        d.element.parentNode.replaceChild(div, d.element);
      }).catch(function(err) {
        console.error('Mermaid render error for ' + d.id + ':', err);
        // Show source on error
        d.element.style.visibility = 'visible';
        d.element.style.height = 'auto';
      });
    });
  };
  document.head.appendChild(s);
});

// === 3. Language selector fix ===
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

// === 4. Click-to-zoom for mermaid diagrams ===
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
