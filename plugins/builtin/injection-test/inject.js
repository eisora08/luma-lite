// Injection Test — LumaForge Lite CEF injection proof-of-concept
// Adds a visible floating button to the Steam Store page.
// Self-contained, idempotent, teardown-safe.
// Survives SPA navigation via MutationObserver + history hooks.
(function () {
  'use strict';

  var EXTENSION_ID = 'injection-test';
  var NAMESPACE = '__lumaforge_injection_test__';
  var BTN_ID = 'luma-injection-test-btn';

  // Prevent double-injection
  if (window[NAMESPACE]) return;
  window[NAMESPACE] = { version: '0.1.0', active: true };

  function log(msg) {
    console.log('[LumaForge InjectionTest] ' + msg);
  }

  function inject() {
    // Skip if already present
    if (document.getElementById(BTN_ID)) return;

    var btn = document.createElement('button');
    btn.id = BTN_ID;
    btn.textContent = 'LF Injection OK';
    btn.title = 'LumaForge injection-test: if you see this, CEF injection works';

    // Inline styles to bypass Steam CSP
    Object.assign(btn.style, {
      position: 'fixed',
      top: '12px',
      right: '12px',
      zIndex: '2147483647',
      background: '#1a9fff',
      color: '#fff',
      border: '2px solid #fff',
      borderRadius: '8px',
      padding: '8px 16px',
      fontSize: '13px',
      fontFamily: 'Arial, sans-serif',
      fontWeight: 'bold',
      cursor: 'pointer',
      boxShadow: '0 2px 12px rgba(0,0,0,0.5)',
      transition: 'transform 0.15s ease'
    });

    // Marker attribute for teardown/detection
    btn.setAttribute('data-lumaforge-extension', EXTENSION_ID);

    btn.addEventListener('mouseenter', function () {
      btn.style.transform = 'scale(1.1)';
    });
    btn.addEventListener('mouseleave', function () {
      btn.style.transform = 'scale(1)';
    });
    btn.addEventListener('click', function () {
      alert(
        '[LumaForge] Injection test button clicked!\n\n' +
        'Extension: ' + EXTENSION_ID + '\n' +
        'Page: ' + window.location.href + '\n' +
        'Timestamp: ' + new Date().toISOString()
      );
    });

    // Try to place near Steam's action bar, otherwise float top-right
    var target =
      document.querySelector('#game_area_purchase_game') ||
      document.querySelector('.game_area_purchase_game') ||
      document.querySelector('.apphub_OtherSiteInfo') ||
      document.querySelector('.queue_controls_ctn');

    if (target) {
      btn.style.position = 'relative';
      btn.style.top = '0';
      btn.style.right = '0';
      btn.style.margin = '8px 0';
      btn.style.display = 'block';
      target.parentNode.insertBefore(btn, target.nextSibling);
    } else {
      document.body.appendChild(btn);
    }

    log('Button injected on ' + window.location.href);
  }

  // Inject immediately if DOM is ready
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', inject);
  } else {
    inject();
  }

  // --- SPA navigation handling (mirrors steam-store-helper pattern) ---

  // 1. MutationObserver on document.documentElement (survives body replacement)
  var lastUrl = location.href;
  var observerRoot = document.documentElement;

  function reattachObserver() {
    if (observer && observerRoot.isConnected) return;
    if (observer) observer.disconnect();
    observerRoot = document.documentElement;
    observer.observe(observerRoot, { childList: true, subtree: true });
  }

  var observer = new MutationObserver(function () {
    if (location.href !== lastUrl) {
      lastUrl = location.href;
      setTimeout(inject, 500);
    }
    // Reattach if root was replaced
    reattachObserver();
  });
  observer.observe(observerRoot, { childList: true, subtree: true });

  // 2. History API hooks (survive any DOM replacement)
  var _origPushState = history.pushState;
  var _origReplaceState = history.replaceState;

  history.pushState = function () {
    _origPushState.apply(this, arguments);
    scheduleReinject();
  };
  history.replaceState = function () {
    _origReplaceState.apply(this, arguments);
    scheduleReinject();
  };

  window.addEventListener('popstate', scheduleReinject);
  window.addEventListener('hashchange', scheduleReinject);

  var reinjectTimer = null;
  function scheduleReinject() {
    if (reinjectTimer) return;
    reinjectTimer = setTimeout(function () {
      reinjectTimer = null;
      if (location.href !== lastUrl) {
        lastUrl = location.href;
        inject();
      }
    }, 300);
  }

  log('Loaded on ' + location.href);
})();
