// Mermaid rendering for Zensical docs
// Problem: Zensical outputs <pre class="mermaid"><code>...encoded HTML...</code></pre>
// Mermaid needs <div class="mermaid">...clean text...</div>

// Hide mermaid source code immediately to prevent flash
document.head.insertAdjacentHTML('beforeend',
  '<style>pre.mermaid{visibility:hidden;height:0;overflow:hidden}</style>');

document.addEventListener('DOMContentLoaded', function() {
  var pres = document.querySelectorAll('pre.mermaid');
  if (pres.length === 0) return;

  // Replace each <pre class="mermaid"><code>...</code></pre>
  // with <div class="mermaid">...decoded text...</div>
  pres.forEach(function(pre) {
    var code = pre.querySelector('code');
    var text = code ? code.textContent : pre.textContent;
    var div = document.createElement('div');
    div.className = 'mermaid';
    div.textContent = text;
    pre.parentNode.replaceChild(div, pre);
  });

  // Load mermaid from CDN
  var s = document.createElement('script');
  s.src = 'https://cdn.jsdelivr.net/npm/mermaid@10/dist/mermaid.min.js';
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
    mermaid.run().then(function() {
      document.querySelectorAll('.mermaid svg').forEach(function(svg) {
        svg.style.cursor = 'zoom-in';
      });
    }).catch(function(err) {
      console.error('Mermaid render error:', err);
    });
  };
  document.head.appendChild(s);
});

// Fix language selector — rewrite links to include current page path
document.addEventListener('DOMContentLoaded', function() {
  var path = location.pathname;
  // Strip language prefix to get the page slug (e.g., /zh-cn/security-model/ -> /security-model/)
  var langPrefixes = ['/zh-cn/', '/es/', '/pt/', '/ja/', '/de/'];
  var pagePath = path;
  langPrefixes.forEach(function(prefix) {
    if (path.startsWith(prefix)) pagePath = path.substring(prefix.length - 1);
  });
  // If on root, pagePath is /
  if (pagePath === '' || pagePath === '/') pagePath = '/';

  document.querySelectorAll('a.md-select__link[hreflang]').forEach(function(a) {
    var lang = a.getAttribute('hreflang');
    var base = a.getAttribute('href'); // e.g., /zh-cn/
    if (lang === 'en') {
      a.setAttribute('href', pagePath);
    } else {
      // Map hreflang to directory name
      var langDir = { zh: 'zh-cn', es: 'es', pt: 'pt', ja: 'ja', de: 'de' }[lang] || lang;
      a.setAttribute('href', '/' + langDir + pagePath);
    }
  });
});

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
