// Steam Store Helper — LumaForge Lite CEF injection payload v2.5.0-download-flow
// Runs inside Steam's Chromium Embedded Framework browser context.
// Self-contained, idempotent, teardown-safe.
// All critical CSS uses inline styles to bypass Steam's CSP.
// Uses event delegation — survives full DOM rebuilds by Steam.
// SPA-aware: MutationObserver + history hooks track app page navigation.
// Button markers: data-lumaforge-extension, data-lumaforge-app-id, data-lumaforge-state
(function () {
  'use strict';

  var LUMA_INJECT_VERSION = '2.5.0-download-flow';
  var DOCUMENT_ID = Date.now() + '-' + Math.random().toString(36).slice(2);

  var EXTENSION_ID = 'steam-store-helper';
  var BRIDGE_HOST = '127.0.0.1';
  var BRIDGE_PORT = 21775;
  var BRIDGE_SCHEME = 'http';
  var NAMESPACE = '__lumaforge_ssh__';
  var BTN_ID = 'luma-action-btn';
  var MODAL_MARKER_ATTR = 'data-lumaforge-modal';
  var MODAL_MARKER_VAL = EXTENSION_ID;
  var BTN_MARKER_ATTR = 'data-lumaforge-extension';
  var BTN_MARKER_VAL = EXTENSION_ID;
  var BTN_APPID_ATTR = 'data-lumaforge-app-id';
  var BTN_STATE_ATTR = 'data-lumaforge-state';
  var APP_URL_RE = /\/app\/(\d+)(?:\/|$)/;
  var MAX_ID_LENGTH = 12;
  var RECONCILE_DEBOUNCE_MS = 200;
  var LOCAL_STATUS_TIMEOUT_MS = 8000;

  var ACTION_SELECTORS = [
    '#game_area_purchase_game .btn_addtocart',
    '.game_area_purchase_game .btn_addtocart',
    '#game_area_purchase_game',
    '.game_area_purchase_game',
    '.apphub_OtherSiteInfo',
    '.app_title_area',
    '.queue_controls_ctn',
    '.game_header_image_full',
  ];

  var _fetchSeq = 0;

  var state = {
    activated: false,
    currentAppId: null,
    currentUrl: null,
    observer: null,
    observerRoot: null,
    historyPatched: false,
    origPushState: null,
    origReplaceState: null,
    statusAbortController: null,
    providerAbortController: null,
    statusRequest: null,

    recoveryTimer: null,
    bridgeRecoveryCount: 0,
    bridgeRecoveryAppId: null,
    statusCache: null,

    retryTimers: [],
    requestContext: null,
    downloadPollTimer: null,
    downloadPollSeq: 0,
    popstateHandler: null,
    hashchangeHandler: null,
    pageshowHandler: null,
    pagehideHandler: null,
    beforeunloadHandler: null,
    _rafPending: false,
    reconcileCount: 0,
    documentId: DOCUMENT_ID,
  };

  // ---------------------------------------------------------------------------
  // SVG Icons (all inline-styled)
  // ---------------------------------------------------------------------------
  function svgDownload(w, h) {
    w = w || 14; h = h || 14;
    return '<svg width="' + w + '" height="' + h + '" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M8 2v8M4.5 7.5L8 11l3.5-3.5"/><path d="M2 12v1.5a1 1 0 001 1h10a1 1 0 001-1V12"/></svg>';
  }
  function svgCloudDownload() {
    return '<svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 16V4M8 12l4 4 4-4"/><path d="M4 18v-2a4 4 0 014-4h8a4 4 0 014 4v2"/></svg>';
  }
  function svgSpinner() {
    return '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" style="animation:luma_ssh_spin .8s linear infinite"><circle cx="12" cy="12" r="10" stroke="currentColor" stroke-width="2" stroke-dasharray="42 42" stroke-linecap="round"/></svg>';
  }
  function svgCheck(w, h) {
    w = w || 14; h = h || 14;
    return '<svg width="' + w + '" height="' + h + '" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 8.5l3.5 3.5L13 4"/></svg>';
  }
  function svgX() {
    return '<svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><path d="M4 4l8 8M12 4l-8 8"/></svg>';
  }
  function svgLibrary() {
    return '<svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="2" width="5" height="12" rx="1"/><rect x="9" y="2" width="5" height="8" rx="1"/></svg>';
  }
  function svgArrowRight() {
    return '<svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M3 8h10M9 4l4 4-4 4"/></svg>';
  }
  function svgErrorCircle() {
    return '<svg width="48" height="48" viewBox="0 0 48 48" fill="none" stroke="#e74c3c" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><circle cx="24" cy="24" r="20"/><path d="M18 18l12 12M30 18L18 30"/></svg>';
  }

  // ---------------------------------------------------------------------------
  // Best-effort keyframe injection
  // ---------------------------------------------------------------------------
  function ensureKeyframes() {
    try {
      if (document.getElementById('luma_ssh_kf')) return;
      var s = document.createElement('style');
      s.id = 'luma_ssh_kf';
      s.textContent =
        '@keyframes luma_ssh_spin{to{transform:rotate(360deg)}}' +
        '@keyframes luma_ssh_fade{from{opacity:0}to{opacity:1}}' +
        '@keyframes luma_ssh_slide{from{opacity:0;transform:translateY(12px)}to{opacity:1;transform:translateY(0)}}';
      (document.head || document.documentElement).appendChild(s);
    } catch (_) { }
  }

  // ---------------------------------------------------------------------------
  // Inline style constants
  // ---------------------------------------------------------------------------
  var ST = {
    btn: 'display:inline-flex;align-items:center;gap:6px;padding:7px 16px;margin-left:10px;border:none;border-radius:3px;cursor:pointer;font-family:inherit;font-size:13px;font-weight:700;letter-spacing:.4px;text-transform:uppercase;background:linear-gradient(to right,#1a9fff,#66c0ff);color:#fff;box-shadow:0 0 8px rgba(102,192,255,.25);vertical-align:middle;position:relative;z-index:1;',
    btnSuccess: 'display:inline-flex;align-items:center;gap:6px;padding:7px 16px;margin-left:10px;border:none;border-radius:3px;cursor:pointer;font-family:inherit;font-size:13px;font-weight:700;letter-spacing:.4px;text-transform:uppercase;background:linear-gradient(to right,#2ea043,#64c882);color:#fff;box-shadow:0 0 8px rgba(100,200,130,.3);vertical-align:middle;position:relative;z-index:1;',
    btnInstalled: 'display:inline-flex;align-items:center;gap:6px;padding:7px 16px;margin-left:10px;border:1px solid rgba(100,200,130,.3);border-radius:3px;cursor:default;font-family:inherit;font-size:13px;font-weight:700;letter-spacing:.4px;text-transform:uppercase;background:rgba(46,160,67,.1);color:#64c882;vertical-align:middle;position:relative;z-index:1;opacity:.7;pointer-events:none;',

    backdrop: 'position:fixed;top:0;left:0;width:100vw;height:100vh;z-index:2147483647;display:flex;align-items:center;justify-content:center;background:rgba(0,0,0,.75);animation:luma_ssh_fade .2s ease-out;font-family:\'Motiva Sans\',Arial,sans-serif;box-sizing:border-box;',
    panel: 'background:#1b2838;color:#c7d5e0;border:1px solid rgba(102,192,255,.15);border-radius:8px;width:480px;max-width:92vw;max-height:80vh;display:flex;flex-direction:column;box-shadow:0 8px 32px rgba(0,0,0,.6),0 0 60px rgba(102,192,255,.08);overflow:hidden;animation:luma_ssh_slide .25s ease-out;box-sizing:border-box;',
    header: 'display:flex;align-items:center;gap:10px;padding:18px 20px 14px;border-bottom:1px solid rgba(102,192,255,.1);',
    headerIcon: 'width:22px;height:22px;color:#66c0ff;flex-shrink:0;display:flex;align-items:center;justify-content:center;',
    headerTitle: 'font-size:16px;font-weight:700;color:#fff;flex:1;margin:0;',
    closeBtn: 'background:none;border:none;cursor:pointer;padding:4px;color:#8f98a0;display:flex;align-items:center;justify-content:center;border-radius:4px;margin-left:auto;',
    body: 'padding:20px;flex:1;overflow-y:auto;min-height:80px;box-sizing:border-box;',
    loading: 'display:flex;flex-direction:column;align-items:center;justify-content:center;gap:14px;padding:24px 0;',
    loadingText: 'font-size:13px;color:#8f98a0;',
    errorWrap: 'text-align:center;padding:20px 0;',
    errorMsg: 'color:#e74c3c;font-size:13px;margin-bottom:4px;',
    errorDetail: 'color:#8f98a0;font-size:11px;margin-top:6px;font-family:monospace;word-break:break-all;max-height:60px;overflow-y:auto;',
    retryBtn: 'background:none;border:1px solid rgba(102,192,255,.3);color:#66c0ff;padding:6px 16px;border-radius:4px;cursor:pointer;font-size:12px;font-weight:600;margin-top:10px;',
    card: 'display:flex;align-items:center;gap:12px;padding:12px 14px;margin-bottom:8px;background:rgba(255,255,255,.03);border:1px solid rgba(255,255,255,.06);border-radius:6px;cursor:pointer;',
    cardIcon: 'width:36px;height:36px;border-radius:6px;background:rgba(102,192,255,.1);display:flex;align-items:center;justify-content:center;flex-shrink:0;color:#66c0ff;',
    cardInfo: 'flex:1;min-width:0;',
    cardName: 'font-size:14px;font-weight:600;color:#fff;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;',
    cardDetail: 'font-size:11px;color:#8f98a0;margin-top:2px;',
    badgeAvail: 'display:inline-flex;align-items:center;gap:4px;padding:4px 10px;border-radius:12px;font-size:11px;font-weight:700;white-space:nowrap;flex-shrink:0;background:rgba(46,160,67,.15);color:#64c882;box-shadow:0 0 8px rgba(46,160,67,.2);',
    badgeUnavail: 'display:inline-flex;align-items:center;gap:4px;padding:4px 10px;border-radius:12px;font-size:11px;font-weight:700;white-space:nowrap;flex-shrink:0;background:rgba(255,255,255,.05);color:#8f98a0;',
    footer: 'padding:14px 20px;border-top:1px solid rgba(102,192,255,.1);display:flex;justify-content:flex-end;',
    cancelBtn: 'background:rgba(255,255,255,.05);border:1px solid rgba(255,255,255,.1);color:#c7d5e0;padding:8px 20px;border-radius:4px;cursor:pointer;font-size:13px;font-weight:600;',
    progressWrap: 'padding:24px 0;text-align:center;',
    progressLabel: 'font-size:13px;color:#8f98a0;margin-bottom:8px;',
    progressBar: 'width:100%;height:6px;background:rgba(255,255,255,.08);border-radius:3px;overflow:hidden;margin:12px 0;',
    progressFill: 'height:100%;background:linear-gradient(to right,#1a9fff,#66c0ff);border-radius:3px;transition:width .3s ease;width:0%;',
    successWrap: 'text-align:center;padding:28px 0 12px;',
    successIcon: 'width:52px;height:52px;border-radius:50%;background:rgba(46,160,67,.15);display:flex;align-items:center;justify-content:center;color:#64c882;margin:0 auto 16px;box-shadow:0 0 20px rgba(46,160,67,.2);',
    successTitle: 'font-size:16px;font-weight:700;color:#fff;margin-bottom:6px;',
    successDetail: 'font-size:12px;color:#8f98a0;margin-bottom:20px;',
    successActions: 'display:flex;gap:12px;justify-content:center;',
    primaryBtn: 'display:inline-flex;align-items:center;gap:6px;padding:10px 22px;border:none;border-radius:4px;cursor:pointer;font-family:inherit;font-size:13px;font-weight:700;letter-spacing:.3px;background:linear-gradient(to right,#1a9fff,#66c0ff);color:#fff;box-shadow:0 0 8px rgba(102,192,255,.25);',
    secondaryBtn: 'display:inline-flex;align-items:center;gap:6px;padding:10px 22px;border:1px solid rgba(102,192,255,.3);border-radius:4px;cursor:pointer;font-family:inherit;font-size:13px;font-weight:600;background:transparent;color:#66c0ff;',
    errorWrap: 'text-align:center;padding:28px 0 12px;',
    errorIcon: 'width:52px;height:52px;border-radius:50%;background:rgba(231,76,60,.12);display:flex;align-items:center;justify-content:center;margin:0 auto 16px;box-shadow:0 0 20px rgba(231,76,60,.15);',
    errorTitle: 'font-size:16px;font-weight:700;color:#fff;margin-bottom:6px;',
    errorMsgNew: 'font-size:12px;color:#e74c3c;margin-bottom:20px;',
    errorActions: 'display:flex;gap:12px;justify-content:center;',
  };

  function dot(hue) {
    var c = hue === 'green' ? '#64c882;box-shadow:0 0 6px rgba(46,160,67,.6)' :
      hue === 'blue' ? '#66c0ff;box-shadow:0 0 6px rgba(102,192,255,.6)' :
        hue === 'red' ? '#e74c3c;box-shadow:0 0 6px rgba(231,76,60,.6)' : '#8f98a0';
    return '<span style="display:inline-block;width:6px;height:6px;border-radius:50%;background:' + c + ';"></span>';
  }

  // ---------------------------------------------------------------------------
  // URL & ID helpers
  // ---------------------------------------------------------------------------
  function extractAppId() {
    try {
      var m = (window.location.pathname || '').match(APP_URL_RE);
      if (!m) m = (window.location.href || '').match(APP_URL_RE);
      if (!m) return null;
      var id = m[1];
      if (!/^\d+$/.test(id) || id.length > MAX_ID_LENGTH || id === '0') return null;
      return id;
    } catch (_) { return null; }
  }

  function findActionContainer() {
    for (var i = 0; i < ACTION_SELECTORS.length; i++) {
      var el = document.querySelector(ACTION_SELECTORS[i]);
      if (el) return el;
    }
    return null;
  }

  function bridgeUrl(path) {
    return BRIDGE_SCHEME + '://' + BRIDGE_HOST + ':' + BRIDGE_PORT + path;
  }

  function sourcesUrl(appId) {
    return bridgeUrl('/api/sources/' + appId);
  }

  function providersUrl() {
    return bridgeUrl('/api/providers');
  }

  function downloadUrl() {
    return bridgeUrl('/api/download');
  }

  function localStatusUrl(appId) {
    return bridgeUrl('/api/local-status/' + appId);
  }

  function downloadStatusUrl(requestId) {
    return bridgeUrl('/api/download-status/' + requestId);
  }

  function openLibraryUrl(appId) {
    return bridgeUrl('/api/open-library/' + appId);
  }

  // ---------------------------------------------------------------------------
  // Bridge fetch helper with comprehensive logging
  // ---------------------------------------------------------------------------
  function bridgeFetch(url, opts, label) {
    console.log('[LUMA_BRIDGE] ' + label + ' — Method:', opts.method || 'GET');
    console.log('[LUMA_BRIDGE] ' + label + ' — URL:', url);
    console.log('[LUMA_BRIDGE] ' + label + ' — Origin:', window.location.origin);
    console.log('[LUMA_BRIDGE] ' + label + ' — Page URL:', window.location.href);
    return fetch(url, opts)
      .then(function (r) {
        console.log('[LUMA_BRIDGE] ' + label + ' — Response status:', r.status, r.statusText);
        return r;
      })
      .catch(function (err) {
        console.error('[LUMA_BRIDGE] ' + label + ' — FAILED:', err.message || err);
        console.error('[LUMA_BRIDGE] ' + label + ' — Error name:', err && err.name);
        console.error('[LUMA_BRIDGE] ' + label + ' — Error message:', err && err.message);
        if (err && err.stack) console.error('[LUMA_BRIDGE] ' + label + ' — Error stack:', err.stack);
        throw err;
      });
  }

  // ---------------------------------------------------------------------------
  // Unified fetch retry helper — retries only network-level failures
  // hooks.beforeAttempt(attempt) -> false to abort the chain
  // ---------------------------------------------------------------------------
  var RETRY_DELAYS = [0, 300, 750, 1500];
  var MAX_RETRY_ATTEMPTS = 4;

  function retryFetch(url, opts, label, hooks) {
    hooks = hooks || {};
    var attempt = 0;

    function tryOnce() {
      attempt++;
      var attemptNum = attempt;

      if (hooks.beforeAttempt && !hooks.beforeAttempt(attemptNum)) {
        return Promise.reject(new DOMException('Context invalidated', 'AbortError'));
      }

      console.log('[LUMA_BRIDGE] ' + label + ' attempt ' + attemptNum + (hooks.appId ? ' for AppID: ' + hooks.appId : ''));

      return fetch(url, opts)
        .then(function (r) {
          if (opts && opts.signal && opts.signal.aborted) {
            throw new DOMException('Aborted', 'AbortError');
          }
          console.log('[LUMA_BRIDGE] ' + label + ' succeeded' + (hooks.appId ? ' for AppID: ' + hooks.appId : ''));
          return r;
        })
        .catch(function (err) {
          if (opts && opts.signal && opts.signal.aborted) {
            throw err;
          }
          if (err && err.name === 'AbortError') {
            throw err;
          }
          var isNetworkFailure = !err || !err.response;
          if (attemptNum < MAX_RETRY_ATTEMPTS && isNetworkFailure) {
            var delay = RETRY_DELAYS[attemptNum] || 1500;
            console.log('[LUMA_BRIDGE] Network failure' + (hooks.appId ? ' for AppID ' + hooks.appId : '') + '; retrying in ' + delay + 'ms');
            return new Promise(function (resolve, reject) {
              var timerId = setTimeout(function () {
                var idx = state.retryTimers.indexOf(timerId);
                if (idx !== -1) state.retryTimers.splice(idx, 1);
                tryOnce().then(resolve, reject);
              }, delay);
              state.retryTimers.push(timerId);
            });
          }
          console.log('[LUMA_BRIDGE] ' + label + ' failed after ' + attemptNum + ' attempts' + (hooks.appId ? ' for AppID: ' + hooks.appId : ''));
          throw err;
        });
    }

    return tryOnce();
  }

  // ---------------------------------------------------------------------------
  // Helper functions
  // ---------------------------------------------------------------------------
  function abortPendingRequests() {
    if (state.statusAbortController) {
      state.statusAbortController.abort();
      state.statusAbortController = null;
    }
    if (state.providerAbortController) {
      state.providerAbortController.abort();
      state.providerAbortController = null;
    }
    if (state.statusRequest) {
      if (state.statusRequest.controller) state.statusRequest.controller.abort();
      if (state.statusRequest.retryTimer) clearTimeout(state.statusRequest.retryTimer);
      state.statusRequest = null;
    }
    if (state.recoveryTimer) {
      clearTimeout(state.recoveryTimer);
      state.recoveryTimer = null;
    }
    stopDownloadPoll();
    cancelAllRetries();
    _fetchSeq++;
  }

  function cancelAllRetries() {
    for (var i = 0; i < state.retryTimers.length; i++) {
      clearTimeout(state.retryTimers[i]);
    }
    state.retryTimers = [];
  }

  function removeButton() {
    var btn = document.getElementById(BTN_ID);
    if (btn) btn.remove();
  }

  function getObserverRoot() {
    if (document.documentElement) return document.documentElement;
    return document.body;
  }

  function syncNamespaceState() {
    if (window[NAMESPACE]) {
      window[NAMESPACE].active = state.activated;
      window[NAMESPACE].currentUrl = state.currentUrl;
      window[NAMESPACE].currentAppId = state.currentAppId;
      window[NAMESPACE].reconcileCount = state.reconcileCount;
      window[NAMESPACE].observerActive = !!state.observer;
      window[NAMESPACE].historyWrapped = state.historyPatched;
    }
  }

  // ---------------------------------------------------------------------------
  // Button state helpers
  // ---------------------------------------------------------------------------
  function setButtonState(appId, style, html, disabled) {
    try {
      var btn = document.getElementById(BTN_ID);
      if (!btn) return false;
      if (btn.getAttribute(BTN_APPID_ATTR) !== appId) return false;
      btn.setAttribute('style', style);
      btn.innerHTML = html;
      if (disabled) {
        btn.setAttribute('aria-disabled', 'true');
      } else {
        btn.removeAttribute('aria-disabled');
      }
      return true;
    } catch (_) { return false; }
  }

  function setButtonLumaState(appId, lumaState) {
    try {
      var btn = document.getElementById(BTN_ID);
      if (!btn) return;
      if (btn.getAttribute(BTN_APPID_ATTR) !== appId) return;
      btn.setAttribute(BTN_STATE_ATTR, lumaState);
    } catch (_) { }
  }

  function applyInLibraryState(appId) {
    setButtonState(appId, ST.btnInstalled, svgCheck() + '<span>IN LIBRARY</span>', true);
    setButtonLumaState(appId, 'in-library');
    console.log('[LUMA_INJECT] App', appId, 'already in Luma library');
  }

  // ---------------------------------------------------------------------------
  // Bridge recovery timer
  // ---------------------------------------------------------------------------
  function scheduleBridgeRecovery(appId) {
    if (state.recoveryTimer) {
      clearTimeout(state.recoveryTimer);
      state.recoveryTimer = null;
    }

    if (!state.activated || state.currentAppId !== appId) {
      return;
    }

    var btn = document.getElementById(BTN_ID);

    if (!btn || btn.getAttribute(BTN_APPID_ATTR) !== appId) {
      return;
    }

    if (btn.getAttribute(BTN_STATE_ATTR) !== 'bridge-error') {
      return;
    }

    if (state.bridgeRecoveryAppId !== appId) {
      state.bridgeRecoveryAppId = appId;
      state.bridgeRecoveryCount = 0;
    }

    if (state.bridgeRecoveryCount >= 1) {
      console.log(
        '[LUMA_BRIDGE] Automatic recovery limit reached for AppID:',
        appId
      );

      return;
    }

    state.bridgeRecoveryCount++;

    console.log(
      '[LUMA_BRIDGE] Scheduling automatic recovery for AppID:',
      appId
    );

    state.recoveryTimer = setTimeout(function () {
      state.recoveryTimer = null;

      if (!state.activated || state.currentAppId !== appId) {
        return;
      }

      var currentButton = document.getElementById(BTN_ID);

      if (
        !currentButton ||
        currentButton.getAttribute(BTN_APPID_ATTR) !== appId
      ) {
        return;
      }

      if (
        currentButton.getAttribute(BTN_STATE_ATTR) !==
        'bridge-error'
      ) {
        return;
      }

      console.log(
        '[LUMA_BRIDGE] Running automatic recovery for AppID:',
        appId
      );

      ensureLumaButtonExists();
    }, 2000);
  }

  // ---------------------------------------------------------------------------
  // Local-status result handler (shared between reconcile and click-to-retry)
  // ---------------------------------------------------------------------------
  function handleLocalStatusResult(appId, err, data) {
    try {
      var b = document.getElementById(BTN_ID);
      if (!b || b.getAttribute(BTN_APPID_ATTR) !== appId) {
        console.log('[LUMA_WATCHER] Button gone or AppID mismatch after local-status for', appId);
        return;
      }

      if (err) {
        if (err.name === 'AbortError') {
          console.log('[LUMA_BRIDGE] Aborted request for AppID:', appId);
          return;
        }
        console.error('[LUMA_INJECT] Bridge error for', appId, ':', err.name || err.message || err);
        setButtonState(appId, ST.btn, svgDownload() + '<span>BRIDGE ERROR</span>', false);
        setButtonLumaState(appId, 'bridge-error');
        scheduleBridgeRecovery(appId);
        return;
      }

      if (!data || !data.ok) {
        var detail = data ? (data.message || 'invalid') : 'no data';
        console.error('[LUMA_INJECT] Invalid status for', appId, ':', detail);
        setButtonState(appId, ST.btn, svgDownload() + '<span>BRIDGE ERROR</span>', false);
        setButtonLumaState(appId, 'bridge-error');
        scheduleBridgeRecovery(appId);
        return;
      }

      state.bridgeRecoveryAppId = appId;
      state.bridgeRecoveryCount = 0;

      var inLibrary = data.inLibrary === true || data.in_library === true;
      console.log('[LUMA_INJECT] App', appId, 'inLibrary =', inLibrary);

      if (!state.statusCache) state.statusCache = {};
      state.statusCache[appId] = { inLibrary: inLibrary, timestamp: Date.now() };

      if (inLibrary) {
        applyInLibraryState(appId);
      } else {
        setButtonState(appId, ST.btn, svgDownload() + '<span>ADD VIA LUMAFORGE</span>', false);
        setButtonLumaState(appId, 'ready');
      }
    } catch (_) { }
  }

  // ---------------------------------------------------------------------------
  // checkLocalStatus — unified with dedup and abortable retry
  // ---------------------------------------------------------------------------
  function checkLocalStatus(appId, cb) {
    try {
      if (state.recoveryTimer) {
        clearTimeout(state.recoveryTimer);
        state.recoveryTimer = null;
      }

      if (state.statusRequest && state.statusRequest.appId === appId) {
        console.log('[LUMA_BRIDGE] Reusing pending request for AppID:', appId);
        return;
      }

      if (state.statusRequest) {
        if (state.statusRequest.controller) state.statusRequest.controller.abort();
        if (state.statusRequest.retryTimer) clearTimeout(state.statusRequest.retryTimer);
        state.statusRequest = null;
      }
      if (state.statusAbortController) {
        state.statusAbortController.abort();
        state.statusAbortController = null;
      }

      var controller = new AbortController();
      state.statusAbortController = controller;
      state.statusRequest = { appId: appId, controller: controller, retryTimer: null };

      var url = localStatusUrl(appId);
      console.log('[LUMA_WATCHER] Local status requested for AppID:', appId);

      retryFetch(url, {
        method: 'GET',
        mode: 'cors',
        cache: 'no-store',
        signal: controller.signal,
      }, 'local-status', {
        appId: appId,
        beforeAttempt: function (attemptNum) {
          if (!state.activated || controller.signal.aborted) return false;
          if (state.currentAppId !== appId) return false;
          var btn = document.getElementById(BTN_ID);
          if (!btn || btn.getAttribute(BTN_APPID_ATTR) !== appId) return false;

          if (attemptNum <= 3) {
            setButtonState(appId, ST.btn + 'opacity:.7;pointer-events:none;', svgSpinner() + '<span>CHECKING\u2026</span>', true);
            setButtonLumaState(appId, 'checking');
          }
          return true;
        },
      })
        .then(function (r) {
          if (controller.signal.aborted) return;
          if (!r.ok) throw new Error('HTTP ' + r.status);
          return r.json();
        })
        .then(function (d) {
          if (controller.signal.aborted) return;
          if (!d || !d.ok) throw new Error('Invalid response');
          var responseAppId = String(d.appId || d.app_id || '');
          if (responseAppId !== String(appId)) {
            console.log('[LUMA_BRIDGE] Stale response ignored for AppID:', appId, '(got:', responseAppId + ')');
            return;
          }
          if (state.currentAppId !== appId) return;

          console.log('[LUMA_BRIDGE] Local-status succeeded for AppID:', appId);

          if (state.statusRequest && state.statusRequest.appId === appId) {
            state.statusRequest = null;
          }

          cb(null, d, appId);
        })
        .catch(function (err) {
          if (err && err.name === 'AbortError') {
            console.log('[LUMA_BRIDGE] Aborted request for AppID:', appId);
            return;
          }
          if (!state.activated || state.currentAppId !== appId) return;

          if (state.statusRequest && state.statusRequest.appId === appId) {
            state.statusRequest = null;
          }

          cb(err, null, appId);
        });
    } catch (e) { cb(e, null, appId); }
  }

  // ---------------------------------------------------------------------------
  // CORE: ensureLumaButtonExists()
  // ---------------------------------------------------------------------------
  function ensureLumaButtonExists() {
    try {
      var appId = extractAppId();

      if (!appId) {
        removeButton();
        state.currentAppId = null;
        return;
      }

      var existing = document.getElementById(BTN_ID);
      if (existing && existing.getAttribute(BTN_APPID_ATTR) === appId) {
        var existingState = existing.getAttribute(BTN_STATE_ATTR);
        if (existingState !== 'bridge-error' && existingState !== 'request-error') {
          return;
        }
        console.log('[LUMA_WATCHER] Retrying for AppID:', appId, '(was:', existingState + ')');
        setButtonState(appId, ST.btn + 'opacity:.7;pointer-events:none;', svgSpinner() + '<span>CHECKING\u2026</span>', true);
        setButtonLumaState(appId, 'checking');
      } else {
        if (existing) existing.remove();

        console.log('[LUMA_WATCHER] Injecting controls for AppID:', appId);

        var container = findActionContainer();
        if (!container) {
          console.log('[LUMA_WATCHER] Target container not found yet for AppID:', appId);
          return;
        }

        var btn = document.createElement('button');
        btn.type = 'button';
        btn.id = BTN_ID;
        btn.setAttribute(BTN_MARKER_ATTR, BTN_MARKER_VAL);
        btn.setAttribute(BTN_APPID_ATTR, appId);
        btn.setAttribute(BTN_STATE_ATTR, 'checking');
        btn.setAttribute('aria-label', 'Add app ' + appId + ' via LumaForge');
        btn.title = 'Select a download source for app ' + appId;
        btn.setAttribute('style', ST.btn + 'opacity:.7;pointer-events:none;');
        btn.setAttribute('aria-disabled', 'true');
        btn.innerHTML = svgSpinner() + '<span>CHECKING\u2026</span>';

        var parent = container.parentNode;
        if (parent) {
          parent.insertBefore(btn, container.nextSibling);
        } else {
          container.appendChild(btn);
        }
      }

      state.currentAppId = appId;
      ensureKeyframes();
      syncNamespaceState();

      console.log('[LUMA_INJECT] Injecting for AppID:', appId);

      var seq = ++_fetchSeq;
      checkLocalStatus(appId, function (err, data, resolvedAppId) {
        if (seq !== _fetchSeq) {
          console.log('[LUMA_WATCHER] Ignored stale response for AppID:', resolvedAppId);
          return;
        }
        if (state.currentAppId !== resolvedAppId) {
          console.log('[LUMA_WATCHER] Ignored stale response for AppID:', resolvedAppId, '(current:', state.currentAppId + ')');
          return;
        }
        handleLocalStatusResult(resolvedAppId, err, data);
      });
    } catch (e) {
      console.error('[CEF_INJECT_ERROR] ensureLumaButtonExists:', e);
    }
  }

  // ---------------------------------------------------------------------------
  // Event delegation
  // ---------------------------------------------------------------------------
  function setupEventDelegation() {
    try {
      document.body.addEventListener('click', function (e) {
        try {
          var btn = e.target.closest('#' + BTN_ID);
          if (btn) {
            e.preventDefault();
            e.stopPropagation();
            var btnAppId = btn.getAttribute(BTN_APPID_ATTR) || extractAppId();
            if (!btnAppId) return;
            var btnLumaState = btn.getAttribute(BTN_STATE_ATTR);
            if (btnLumaState === 'bridge-error' || btnLumaState === 'request-error') {
              console.log('[LUMA_INJECT] Retry click for AppID:', btnAppId);

              state.bridgeRecoveryAppId = btnAppId;
              state.bridgeRecoveryCount = 0;

              setButtonState(btnAppId, ST.btn + 'opacity:.7;pointer-events:none;', svgSpinner() + '<span>CHECKING\u2026</span>', true);
              setButtonLumaState(btnAppId, 'checking');
              checkLocalStatus(btnAppId, function (err, data, resolvedId) {
                handleLocalStatusResult(resolvedId, err, data);
              });
              return;
            }
            if (btn.getAttribute('aria-disabled') === 'true') return;
            var btnText = btn.textContent || '';
            if (btnText.indexOf('IN LIBRARY') !== -1) return;
            console.log('[LUMA_INJECT] Delegated click for AppID:', btnAppId);
            openSourceModal(btnAppId);
            return;
          }
          if (e.target.id === 'luma-retry-sources' || e.target.closest('#luma-retry-sources')) {
            e.preventDefault();
            e.stopPropagation();
            var retryAppId = extractAppId();
            if (retryAppId) openSourceModal(retryAppId);
            return;
          }
        } catch (err) {
          console.error('[CEF_INJECT_ERROR] delegated click:', err);
        }
      }, true);

      document.body.addEventListener('click', function (e) {
        try {
          if (e.target.getAttribute(MODAL_MARKER_ATTR) === MODAL_MARKER_VAL) {
            closeModal();
          }
        } catch (_) { }
      }, true);

      document.addEventListener('keydown', function (e) {
        try {
          if (e.key === 'Escape' || e.keyCode === 27) {
            var modal = document.querySelector('[' + MODAL_MARKER_ATTR + '="' + MODAL_MARKER_VAL + '"]');
            if (modal) {
              e.preventDefault();
              e.stopPropagation();
              closeModal();
            }
          }
        } catch (_) { }
      }, true);

      console.log('[LUMA_INJECT] Event delegation active on document.body');
    } catch (e) {
      console.error('[CEF_INJECT_ERROR] setupEventDelegation:', e);
    }
  }

  // ---------------------------------------------------------------------------
  // Modal: close
  // ---------------------------------------------------------------------------
  function closeModal() {
    try {
      if (state.providerAbortController) {
        state.providerAbortController.abort();
        state.providerAbortController = null;
      }
      stopDownloadPoll();
      var m = document.querySelector('[' + MODAL_MARKER_ATTR + '="' + MODAL_MARKER_VAL + '"]');
      if (m) m.remove();
    } catch (e) { console.error('[CEF_INJECT_ERROR] closeModal:', e); }
  }

  // ---------------------------------------------------------------------------
  // Modal: build and show
  // ---------------------------------------------------------------------------
  function openSourceModal(appId) {
    try {
      console.log('[LUMA_INJECT] Opening modal for AppID:', appId);

      closeModal();

      var backdrop = document.createElement('div');
      backdrop.setAttribute(MODAL_MARKER_ATTR, MODAL_MARKER_VAL);
      backdrop.setAttribute('style', ST.backdrop);

      var panel = document.createElement('div');
      panel.setAttribute('style', ST.panel);
      panel.addEventListener('click', function (e) { e.stopPropagation(); });

      var header = document.createElement('div');
      header.setAttribute('style', ST.header);
      var hdrIcon = document.createElement('span');
      hdrIcon.setAttribute('style', ST.headerIcon);
      hdrIcon.innerHTML = svgCloudDownload();
      var hdrTitle = document.createElement('span');
      hdrTitle.setAttribute('style', ST.headerTitle);
      hdrTitle.textContent = 'Select Download Source';

      var hdrVersion = document.createElement('div');
      hdrVersion.setAttribute('style', 'font-size:10px;color:#8f98a0;margin-top:2px;');
      hdrVersion.textContent = 'Runtime: ' + LUMA_INJECT_VERSION;
      var closeBtn = document.createElement('button');
      closeBtn.setAttribute('style', ST.closeBtn);
      closeBtn.setAttribute('aria-label', 'Close');
      closeBtn.innerHTML = svgX();
      closeBtn.addEventListener('click', closeModal);
      header.appendChild(hdrIcon);
      header.appendChild(hdrTitle);
      header.appendChild(hdrVersion);
      header.appendChild(closeBtn);

      var body = document.createElement('div');
      body.setAttribute('style', ST.body);
      body.innerHTML =
        '<div style="' + ST.loading + '">' +
        svgSpinner() +
        '<span style="' + ST.loadingText + '">Loading providers\u2026</span>' +
        '</div>';

      var footer = document.createElement('div');
      footer.setAttribute('style', ST.footer);
      var cancelBtn = document.createElement('button');
      cancelBtn.setAttribute('style', ST.cancelBtn);
      cancelBtn.textContent = 'Cancel';
      cancelBtn.addEventListener('click', closeModal);
      footer.appendChild(cancelBtn);

      panel.appendChild(header);
      panel.appendChild(body);
      panel.appendChild(footer);
      backdrop.appendChild(panel);

      (document.body || document.documentElement).appendChild(backdrop);
      console.log('[LUMA_INJECT] Modal appended to document.body');

      console.log("[PROVIDER_FETCH] VERSION:", LUMA_INJECT_VERSION);
      console.log("[PROVIDER_FETCH] METHOD:", "GET");
      console.log("[PROVIDER_FETCH] URL:", sourcesUrl(appId));
      console.log("[PROVIDER_FETCH] PAGE:", window.location.href);
      console.log("[PROVIDER_FETCH] ORIGIN:", window.location.origin);
      console.log("[PROVIDER_FETCH] PROTOCOL:", window.location.protocol);
      console.log("[PROVIDER_FETCH] SECURE_CONTEXT:", window.isSecureContext);

      if (state.providerAbortController) {
        state.providerAbortController.abort();
      }
      var providerController = new AbortController();
      state.providerAbortController = providerController;

      var url = sourcesUrl(appId);
      var providerOpts = { method: 'GET', mode: 'cors', cache: 'no-store', signal: providerController.signal };
      retryFetch(url, providerOpts, 'sources', {
        appId: appId,
        beforeAttempt: function () {
          if (!state.activated || providerController.signal.aborted) return false;
          var modal = document.querySelector('[' + MODAL_MARKER_ATTR + '="' + MODAL_MARKER_VAL + '"]');
          if (!modal) return false;
          return true;
        },
      })
        .then(function (r) {
          console.log("[PROVIDER_FETCH] RESPONSE_RECEIVED:", true);
          console.log("[PROVIDER_FETCH] STATUS:", r.status);
          console.log("[PROVIDER_FETCH] OK:", r.ok);
          if (!r.ok) throw new Error('HTTP ' + r.status);
          return r.json();
        })
        .then(function (d) {
          if (providerController.signal.aborted) return;

          var sources = Array.isArray(d.sources) ? d.sources : [];
          var unavailableSources = Array.isArray(d.unavailableSources)
            ? d.unavailableSources
            : [];

          console.log('[LUMA_BRIDGE] sources — Received', sources.length, 'sources');
          if (d && d.ok && Array.isArray(sources)) {

            renderSources(
              body,
              sources,
              unavailableSources,
              appId,
              d.message || null
            );

          } else {
            throw new Error((d && d.message) || 'Invalid response from bridge');
          }
        })
        .catch(function (err) {
          if (err && err.name === 'AbortError') return;
          console.error("[PROVIDER_FETCH] REJECTED:", err);
          console.error("[PROVIDER_FETCH] ERROR_NAME:", err && err.name);
          console.error("[PROVIDER_FETCH] ERROR_MESSAGE:", err && err.message);
          console.error("[PROVIDER_FETCH] ERROR_STACK:", err && err.stack);
          console.error('[LUMA_BRIDGE] providers — Error:', err.name || err.message || err);
          var detail = (err.name || 'Error') + ': ' + (err.message || '');
          body.innerHTML =
            '<div style="' + ST.errorWrap + '">' +
            '<div style="' + ST.errorMsg + '">Could not load providers.</div>' +
            '<div style="' + ST.errorDetail + '">' + detail + '</div>' +
            '<div style="text-align:center"><button id="luma-retry-sources" style="' + ST.retryBtn + '">Retry</button></div>' +
            '</div>';
        });
    } catch (e) { console.error('[CEF_INJECT_ERROR] openSourceModal:', e); }
  }

  // ---------------------------------------------------------------------------
  // Modal: render source list
  // ---------------------------------------------------------------------------
  function renderSources(body, sources, appId, unavailableSources, message) {
    try {
      if (!sources || !sources.length) {
        var unavailableDetails = '';

        if (unavailableSources && unavailableSources.length) {
          unavailableDetails =
            '<div style="margin-top:12px;text-align:left;">' +
            unavailableSources.map(function (source) {
              var name = source.name || source.id || 'Provider';
              var reason = source.detail || 'Package not available';

              return (
                '<div style="margin-top:6px;padding:8px 10px;' +
                'background:rgba(255,255,255,.03);' +
                'border:1px solid rgba(255,255,255,.06);' +
                'border-radius:4px;">' +
                '<div style="color:#c7d5e0;font-size:12px;font-weight:600;">' +
                name +
                '</div>' +
                '<div style="color:#8f98a0;font-size:11px;margin-top:2px;">' +
                reason +
                '</div>' +
                '</div>'
              );
            }).join('') +
            '</div>';
        }

        body.innerHTML =
          '<div style="' + ST.errorWrap + '">' +
          '<div style="' + ST.errorMsg + '">' +
          'No package sources available.' +
          '</div>' +
          '<div style="' + ST.errorDetail + '">' +
          (message ||
            'No enabled provider currently has a package for this App ID.') +
          '</div>' +
          unavailableDetails +
          '</div>';

        return;
      }

      body.innerHTML = '';
      sources.forEach(function (src) {
        var avail = !!src.available;

        var card = document.createElement('div');
        card.setAttribute('style', avail ? ST.card : ST.card + 'opacity:.45;cursor:default;');
        card.setAttribute('data-lumaforge-source-id', src.id);

        var icon = document.createElement('div');
        icon.setAttribute('style', ST.cardIcon);
        icon.innerHTML = svgDownload(18, 18);

        var info = document.createElement('div');
        info.setAttribute('style', ST.cardInfo);
        var name = document.createElement('div');
        name.setAttribute('style', ST.cardName);
        name.textContent = src.name || src.id;
        var detail = document.createElement('div');
        detail.setAttribute('style', ST.cardDetail);
        if (avail) {
          var fileText = (src.files || 0) + ' file' + ((src.files || 0) !== 1 ? 's' : '');
          detail.textContent = 'Ready to download \u2022 ' + fileText;
        } else {
          detail.textContent = src.detail || 'Not available';
        }
        info.appendChild(name);
        info.appendChild(detail);

        var badge = document.createElement('div');
        badge.setAttribute('style', avail ? ST.badgeAvail : ST.badgeUnavail);
        badge.innerHTML = avail
          ? dot('green') + '<span>Ready</span>'
          : dot('gray') + '<span>Unavailable</span>';

        card.appendChild(icon);
        card.appendChild(info);
        card.appendChild(badge);

        if (avail) {
          card.addEventListener('click', function () {
            handleSourceClick(card, appId, src.id);
          });
        }

        body.appendChild(card);
      });

      // Output type selection
      var outputWrap = document.createElement('div');
      outputWrap.setAttribute('style', 'margin-top:12px;padding:10px 14px;background:rgba(255,255,255,.03);border:1px solid rgba(255,255,255,.06);border-radius:6px;');
      var outputLabel = document.createElement('div');
      outputLabel.setAttribute('style', 'font-size:12px;color:#8f98a0;margin-bottom:6px;font-weight:600;');
      outputLabel.textContent = 'Output type:';
      outputWrap.appendChild(outputLabel);


      var outputTypes = [
        { value: 'lua+manifest', label: 'Lua + Manifest' },
        { value: 'lua', label: 'Lua only' },
        { value: 'manifest', label: 'Manifest only' },
      ];

      var outputGroup = document.createElement('div');
      outputGroup.setAttribute('style', 'display:flex;gap:12px;');
      outputTypes.forEach(function (opt, idx) {
        var lbl = document.createElement('label');
        lbl.setAttribute('style', 'display:inline-flex;align-items:center;gap:4px;font-size:12px;color:#c7d5e0;cursor:pointer;');
        var radio = document.createElement('input');
        radio.type = 'radio';
        radio.name = 'luma-output-type';
        radio.value = opt.value;
        if (idx === 0) radio.checked = true;
        radio.setAttribute('style', 'margin:0;');
        lbl.appendChild(radio);
        lbl.appendChild(document.createTextNode(opt.label));
        outputGroup.appendChild(lbl);
      });
      outputWrap.appendChild(outputGroup);
      body.appendChild(outputWrap);
    } catch (e) { console.error('[CEF_INJECT_ERROR] renderSources:', e); }
  }

  // ---------------------------------------------------------------------------
  // Modal: source click -> download with state machine
  // ---------------------------------------------------------------------------
  function handleSourceClick(card, appId, sourceId) {
    try {
      if (card.getAttribute('data-pending') === 'true') return;
      card.setAttribute('data-pending', 'true');
      card.setAttribute('style', ST.card + 'opacity:.5;cursor:wait;pointer-events:none;');

      var badge = card.querySelector('div:last-child');
      if (badge) {
        badge.setAttribute('style', ST.badgeAvail);
        badge.innerHTML = dot('blue') + '<span>ADDING\u2026</span>';
      }

      setButtonState(appId, ST.btn + 'opacity:.7;pointer-events:none;', svgSpinner() + '<span>ADDING\u2026</span>', true);
      setButtonLumaState(appId, 'adding');

      var selectedOutputType = 'lua+manifest';
      var outputRadio = document.querySelector('input[name="luma-output-type"]:checked');
      if (outputRadio) selectedOutputType = outputRadio.value;

      var payload = JSON.stringify({ appId: appId, sourceId: sourceId, outputType: selectedOutputType });
      bridgeFetch(downloadUrl(), {
        method: 'POST',
        mode: 'cors',
        cache: 'no-store',
        headers: { 'Content-Type': 'application/json' },
        body: payload,
      }, 'download')
        .then(function (r) {
          if (!r.ok) throw new Error('HTTP ' + r.status);
          return r.json();
        })
        .then(function (d) {
          if (!d || !d.ok) throw new Error((d && d.message) || 'Download rejected');
          console.log('[LUMA_INJECT] Download accepted for', appId, 'via', sourceId, 'requestId:', d.requestId);

          state.requestContext = {
            requestId: d.requestId,
            appId: appId,
            sourceId: sourceId,
          };

          if (badge) {
            badge.setAttribute('style', ST.badgeAvail);
            badge.innerHTML = dot('blue') + '<span>Queued</span>';
          }

          showDownloadProgress(appId, d.requestId);
          startDownloadPoll(d.requestId, appId);
        })
        .catch(function (err) {
          console.error('[LUMA_BRIDGE] download — Error:', err.message || err);
          card.setAttribute('data-pending', 'false');
          card.setAttribute('style', ST.card);
          if (badge) {
            badge.setAttribute('style', ST.badgeUnavail);
            badge.innerHTML = dot('red') + '<span>FAILED \u2014 RETRY</span>';
          }
          setButtonState(appId, ST.btn, svgDownload() + '<span>TRY AGAIN</span>', false);
          setButtonLumaState(appId, 'ready');
          setTimeout(function () {
            setButtonState(appId, ST.btn, svgDownload() + '<span>ADD VIA LUMAFORGE</span>', false);
          }, 3000);
        });
    } catch (e) { console.error('[CEF_INJECT_ERROR] handleSourceClick:', e); }
  }

  // ---------------------------------------------------------------------------
  // Download progress: show progress state in modal
  // ---------------------------------------------------------------------------
  function showDownloadProgress(appId, requestId) {
    try {
      var body = document.querySelector('[' + MODAL_MARKER_ATTR + '="' + MODAL_MARKER_VAL + '"] > div > div:nth-child(2)');
      if (!body) return;

      body.innerHTML =
        '<div style="' + ST.progressWrap + '">' +
        '<div style="margin-bottom:14px;">' + svgSpinner() + '</div>' +
        '<div style="font-size:14px;font-weight:600;color:#fff;margin-bottom:6px;">Downloading\u2026</div>' +
        '<div style="' + ST.progressLabel + '">Request ' + (requestId || '').slice(0, 20) + '</div>' +
        '<div style="' + ST.progressBar + '"><div id="luma-progress-fill" style="' + ST.progressFill + '"></div></div>' +
        '<div id="luma-progress-status" style="font-size:12px;color:#66c0ff;">Queued</div>' +
        '</div>';
    } catch (_) { }
  }

  function updateProgressUI(job) {
    try {
      var fill = document.getElementById('luma-progress-fill');
      var status = document.getElementById('luma-progress-status');
      if (fill) {
        var pct = job.progress;
        if (!pct && pct !== 0) {
          if (job.status === 'completed') pct = 100;
          else if (job.status === 'processing') pct = 90;
          else if (job.status === 'extracting') pct = 75;
          else if (job.status === 'downloading') pct = 50;
          else if (job.status === 'checking_availability') pct = 20;
          else if (job.status === 'validating') pct = 5;
          else if (job.status === 'queued') pct = 0;
          else pct = 10;
        }
        fill.style.width = pct + '%';
      }
      if (status) {
        var msg = job.message || job.status || 'Working\u2026';
        status.textContent = msg;
      }
    } catch (_) { }
  }

  // ---------------------------------------------------------------------------
  // Download poll: poll GET /api/download-status/{requestId}
  // ---------------------------------------------------------------------------
  function startDownloadPoll(requestId, appId) {
    stopDownloadPoll();
    state.downloadPollSeq++;
    var seq = state.downloadPollSeq;

    function poll() {
      if (!state.requestContext || state.requestContext.requestId !== requestId) return;
      if (!state.activated || state.currentAppId !== appId) return;

      console.log('[LUMA_INJECT] Polling download status for requestId:', requestId);

      fetch(downloadStatusUrl(requestId), {
        method: 'GET',
        mode: 'cors',
        cache: 'no-store',
      })
        .then(function (r) {
          if (!r.ok) throw new Error('HTTP ' + r.status);
          return r.json();
        })
        .then(function (d) {
          if (state.downloadPollSeq !== seq) return;
          if (!d || !d.ok) throw new Error((d && d.message) || 'Invalid response');

          console.log('[LUMA_INJECT] Download status:', d.status, 'for requestId:', requestId);
          updateProgressUI(d);

          if (d.status === 'completed') {
            state.requestContext = null;
            showDownloadSuccess(appId, requestId);
            return;
          }
          if (d.status === 'failed') {
            state.requestContext = null;
            showDownloadError(appId, d.message || 'Download failed', d.errorCode);
            return;
          }

          state.downloadPollTimer = setTimeout(poll, 1500);
        })
        .catch(function (err) {
          if (state.downloadPollSeq !== seq) return;
          console.error('[LUMA_INJECT] Poll error:', err.message || err);
          state.downloadPollTimer = setTimeout(poll, 2500);
        });
    }

    state.downloadPollTimer = setTimeout(poll, 800);
  }

  function stopDownloadPoll() {
    state.downloadPollSeq++;
    if (state.downloadPollTimer) {
      clearTimeout(state.downloadPollTimer);
      state.downloadPollTimer = null;
    }
  }

  // ---------------------------------------------------------------------------
  // Download success: show success state in modal
  // ---------------------------------------------------------------------------
  function showDownloadSuccess(appId, requestId) {
    try {
      var body = document.querySelector('[' + MODAL_MARKER_ATTR + '="' + MODAL_MARKER_VAL + '"] > div > div:nth-child(2)');
      if (!body) return;

      setButtonState(appId, ST.btnSuccess, svgCheck() + '<span>ADDED TO LUMAFORGE</span>', false);
      setButtonLumaState(appId, 'added');

      body.innerHTML =
        '<div style="' + ST.successWrap + '">' +
        '<div style="' + ST.successIcon + '">' + svgCheck(26, 26) + '</div>' +
        '<div style="' + ST.successTitle + '">Package Added Successfully</div>' +
        '<div style="' + ST.successDetail + '">The package has been downloaded and installed to your Steam library.</div>' +
        '<div style="' + ST.successActions + '">' +
        '<button id="luma-btn-open-library" style="' + ST.primaryBtn + '">' + svgLibrary() + '<span>VIEW IN LIBRARY</span></button>' +
        '<button id="luma-btn-continue" style="' + ST.secondaryBtn + '">CONTINUE BROWSING</button>' +
        '</div>' +
        '</div>';

      var openLibBtn = document.getElementById('luma-btn-open-library');
      if (openLibBtn) {
        openLibBtn.addEventListener('click', function () {
          fetch(openLibraryUrl(appId), { method: 'POST', mode: 'cors', cache: 'no-store' })
            .catch(function () { });
          closeModal();
        });
      }

      var continueBtn = document.getElementById('luma-btn-continue');
      if (continueBtn) {
        continueBtn.addEventListener('click', function () {
          closeModal();
        });
      }
    } catch (_) { }
  }

  // ---------------------------------------------------------------------------
  // Download error: show error state in modal
  // ---------------------------------------------------------------------------
  function showDownloadError(appId, message, errorCode) {
    try {
      var body = document.querySelector('[' + MODAL_MARKER_ATTR + '="' + MODAL_MARKER_VAL + '"] > div > div:nth-child(2)');
      if (!body) return;

      setButtonState(appId, ST.btn, svgDownload() + '<span>TRY AGAIN</span>', false);
      setButtonLumaState(appId, 'ready');
      setTimeout(function () {
        setButtonState(appId, ST.btn, svgDownload() + '<span>ADD VIA LUMAFORGE</span>', false);
      }, 4000);

      var detail = errorCode ? (errorCode + ': ' + message) : message;
      body.innerHTML =
        '<div style="' + ST.errorWrap + '">' +
        '<div style="' + ST.errorIcon + '">' + svgErrorCircle() + '</div>' +
        '<div style="' + ST.errorTitle + '">Download Failed</div>' +
        '<div style="' + ST.errorMsgNew + '">' + detail + '</div>' +
        '<div style="' + ST.errorActions + '">' +
        '<button id="luma-btn-retry-download" style="' + ST.primaryBtn + '">' + svgDownload() + '<span>TRY AGAIN</span></button>' +
        '<button id="luma-btn-close-error" style="' + ST.secondaryBtn + '">CLOSE</button>' +
        '</div>' +
        '</div>';

      var retryBtn = document.getElementById('luma-btn-retry-download');
      if (retryBtn) {
        retryBtn.addEventListener('click', function () {
          openSourceModal(appId);
        });
      }

      var closeBtn = document.getElementById('luma-btn-close-error');
      if (closeBtn) {
        closeBtn.addEventListener('click', function () {
          closeModal();
        });
      }
    } catch (_) { }
  }

  // ---------------------------------------------------------------------------
  // Reconcile: detect URL/AppID changes and update controls
  // ---------------------------------------------------------------------------
  function reconcile() {
    state.reconcileCount++;
    var url = window.location.href;
    var appId = extractAppId();
    var prevUrl = state.currentUrl;
    var prevAppId = state.currentAppId;

    console.log('[LUMA_WATCHER] Reconcile start');
    console.log('[LUMA_WATCHER] Current URL:', url);
    console.log('[LUMA_WATCHER] Previous URL:', prevUrl);
    console.log('[LUMA_WATCHER] Parsed AppID:', appId);
    console.log('[LUMA_WATCHER] Previous AppID:', prevAppId);
    console.log('[LUMA_WATCHER] Document ID:', state.documentId);

    state.currentUrl = url;

    // Check observer root connectivity
    if (state.observer && state.observerRoot) {
      var connected = state.observerRoot.isConnected;
      console.log('[LUMA_WATCHER] Observer root connected:', connected);
      if (!connected) {
        console.log('[LUMA_WATCHER] Reattached observer after root replacement');
        state.observer.disconnect();
        state.observerRoot = getObserverRoot();
        state.observer.observe(state.observerRoot, { childList: true, subtree: true });
      }
    }

    // Not on an app page
    if (!appId) {
      if (prevAppId) {
        console.log('[LUMA_WATCHER] Left app page, removing controls for AppID:', prevAppId);
        abortPendingRequests();
        removeButton();
        closeModal();
        state.currentAppId = null;
        syncNamespaceState();
      }
      return;
    }

    // App ID changed
    if (appId !== prevAppId) {
      console.log(
        '[LUMA_WATCHER] AppID changed:',
        prevAppId,
        '->',
        appId
      );

      abortPendingRequests();
      removeButton();
      closeModal();

      state.currentAppId = appId;
      state.bridgeRecoveryAppId = appId;
      state.bridgeRecoveryCount = 0;

      ensureLumaButtonExists();
      syncNamespaceState();
      return;
    }

    // Same App ID — check if button still exists and its state
    var existingBtn = document.getElementById(BTN_ID);
    if (!existingBtn || existingBtn.getAttribute(BTN_APPID_ATTR) !== appId) {
      console.log('[LUMA_WATCHER] Button missing for same AppID:', appId);
      ensureLumaButtonExists();
      syncNamespaceState();
      return;
    }

    var existingState = existingBtn.getAttribute(BTN_STATE_ATTR);
    console.log('[LUMA_WATCHER] Existing button state:', existingState);

    if (existingState === 'bridge-error' || existingState === 'request-error') {
      if (state.recoveryTimer) {
        console.log('[LUMA_WATCHER] Button in retryable state, recovery already scheduled for AppID:', appId);
      } else {
        console.log('[LUMA_WATCHER] Button in retryable state, scheduling recovery for AppID:', appId);
        scheduleBridgeRecovery(appId);
      }
      syncNamespaceState();
      return;
    }

    syncNamespaceState();
  }

  // ---------------------------------------------------------------------------
  // MutationObserver
  // ---------------------------------------------------------------------------
  function startObserver() {
    try {
      if (state.observer) {
        if (state.observerRoot && !state.observerRoot.isConnected) {
          console.log('[LUMA_WATCHER] Observer root disconnected, reattaching');
          state.observer.disconnect();
          state.observerRoot = getObserverRoot();
          state.observer.observe(state.observerRoot, { childList: true, subtree: true });
          console.log('[LUMA_WATCHER] Reattached observer after root replacement');
        }
        return;
      }
      state.observerRoot = getObserverRoot();
      state.observer = new MutationObserver(function () {
        scheduleReconcile('dom');
      });
      state.observer.observe(state.observerRoot, { childList: true, subtree: true });
      console.log('[LUMA_WATCHER] MutationObserver attached to', state.observerRoot.nodeName);
      console.log('[LUMA_WATCHER] Observer root:', state.observerRoot.nodeName);
    } catch (e) { console.error('[CEF_INJECT_ERROR] startObserver:', e); }
  }

  function stopObserver() {
    try {
      if (state.observer) { state.observer.disconnect(); state.observer = null; state.observerRoot = null; }
    } catch (e) { console.error('[CEF_INJECT_ERROR] stopObserver:', e); }
  }

  function scheduleReconcile(reason) {
    try {
      if (!state.activated || state._rafPending) return;
      state._rafPending = true;
      console.log('[LUMA_WATCHER] DOM reconciliation requested:', reason);
      requestAnimationFrame(function () {
        state._rafPending = false;
        if (!state.activated) return;
        reconcile();
      });
    } catch (e) { console.error('[CEF_INJECT_ERROR] scheduleReconcile:', e); }
  }

  // ---------------------------------------------------------------------------
  // SPA navigation hooks
  // ---------------------------------------------------------------------------
  function patchHistory() {
    try {
      if (state.historyPatched) return;
      state.origPushState = History.prototype.pushState;
      state.origReplaceState = History.prototype.replaceState;
      History.prototype.pushState = function () {
        var r = state.origPushState.apply(this, arguments);
        console.log('[LUMA_WATCHER] pushState');
        scheduleReconcile('pushState');
        return r;
      };
      History.prototype.replaceState = function () {
        var r = state.origReplaceState.apply(this, arguments);
        console.log('[LUMA_WATCHER] replaceState');
        scheduleReconcile('replaceState');
        return r;
      };
      state.popstateHandler = function () {
        console.log('[LUMA_WATCHER] popstate');
        scheduleReconcile('popstate');
      };
      state.hashchangeHandler = function () {
        console.log('[LUMA_WATCHER] hashchange');
        scheduleReconcile('hashchange');
      };
      state.pageshowHandler = function () {
        console.log('[LUMA_WATCHER] pageshow');
        scheduleReconcile('pageshow');
      };
      state.pagehideHandler = function () {
        console.log('[LUMA_WATCHER] pagehide');
      };
      state.beforeunloadHandler = function () {
        console.log('[LUMA_WATCHER] beforeunload');
      };
      window.addEventListener('popstate', state.popstateHandler);
      window.addEventListener('hashchange', state.hashchangeHandler);
      window.addEventListener('pageshow', state.pageshowHandler);
      window.addEventListener('pagehide', state.pagehideHandler);
      window.addEventListener('beforeunload', state.beforeunloadHandler);
      state.historyPatched = true;
      console.log('[LUMA_WATCHER] SPA navigation hooks installed');
    } catch (e) { console.error('[CEF_INJECT_ERROR] patchHistory:', e); }
  }

  function restoreHistory() {
    try {
      if (!state.historyPatched) return;
      if (state.origPushState) History.prototype.pushState = state.origPushState;
      if (state.origReplaceState) History.prototype.replaceState = state.origReplaceState;
      if (state.popstateHandler) window.removeEventListener('popstate', state.popstateHandler);
      if (state.hashchangeHandler) window.removeEventListener('hashchange', state.hashchangeHandler);
      if (state.pageshowHandler) window.removeEventListener('pageshow', state.pageshowHandler);
      if (state.pagehideHandler) window.removeEventListener('pagehide', state.pagehideHandler);
      if (state.beforeunloadHandler) window.removeEventListener('beforeunload', state.beforeunloadHandler);
      state.popstateHandler = null;
      state.hashchangeHandler = null;
      state.pageshowHandler = null;
      state.pagehideHandler = null;
      state.beforeunloadHandler = null;
      state.origPushState = null;
      state.origReplaceState = null;
      state.historyPatched = false;
    } catch (e) { console.error('[CEF_INJECT_ERROR] restoreHistory:', e); }
  }

  // ---------------------------------------------------------------------------
  // Teardown
  // ---------------------------------------------------------------------------
  function teardown() {
    try {
      console.log('[LUMA_WATCHER] Tearing down lifecycle');
      state.activated = false;
      abortPendingRequests();
      cancelAllRetries();
      stopDownloadPoll();
      state.requestContext = null;
      if (state.providerAbortController) {
        state.providerAbortController.abort();
        state.providerAbortController = null;
      }
      state.statusCache = null;
      stopObserver();
      restoreHistory();
      closeModal();
      removeButton();
      state.currentAppId = null;
      state.currentUrl = null;
      syncNamespaceState();
    } catch (e) { console.error('[CEF_INJECT_ERROR] teardown:', e); }
  }

  // ---------------------------------------------------------------------------
  // Activation
  // ---------------------------------------------------------------------------
  function activate() {
    try {
      if (state.activated) {
        console.log('[LUMA_RUNTIME] Already active, scheduling reconcile');
        reconcile();
        return;
      }
      state.activated = true;
      state.currentUrl = location.href;
      state.currentAppId = extractAppId();
      patchHistory();
      setupEventDelegation();
      startObserver();
      ensureLumaButtonExists();
      syncNamespaceState();
      console.log('[LUMA_RUNTIME] Version:', LUMA_INJECT_VERSION);
      console.log('[LUMA_RUNTIME] Target URL:', window.location.href);
      console.log('[LUMA_RUNTIME] Document ID:', state.documentId);
      console.log('[LUMA_RUNTIME] Lifecycle already existed: false');
      console.log('[LUMA_RUNTIME] Existing lifecycle version: none');
      console.log('[LUMA_RUNTIME] Active:', true);
      console.log('[LUMA_RUNTIME] Current AppID:', state.currentAppId);
      console.log('[LUMA_RUNTIME] Reconcile count:', state.reconcileCount);
      console.log('[LUMA_RUNTIME] Observer active:', !!state.observer);
      console.log('[LUMA_RUNTIME] History wrapped:', state.historyPatched);
    } catch (e) { console.error('[CEF_INJECT_ERROR] activate:', e); }
  }

  // ---------------------------------------------------------------------------
  // Namespace & Bootstrap
  // ---------------------------------------------------------------------------
  try {
    console.log('[FORENSIC_INJECT] VERSION=' + LUMA_INJECT_VERSION);
    console.log('[FORENSIC_INJECT] PAGE_URL=' + window.location.href);
    console.log('[FORENSIC_INJECT] ORIGIN=' + window.location.origin);
    console.log('[FORENSIC_INJECT] Document ID=' + DOCUMENT_ID);

    var existingLifecycle = !!(window[NAMESPACE] && window[NAMESPACE].activate);
    var existingVersion = (window[NAMESPACE] && window[NAMESPACE].version) || 'none';

    console.log('[FORENSIC_INJECT] Existing lifecycle found: ' + existingLifecycle);
    console.log('[FORENSIC_INJECT] Existing version: ' + existingVersion);

    // Same-version reuse: if same version is already active, just reconcile
    if (window[NAMESPACE] && window[NAMESPACE].version === LUMA_INJECT_VERSION && window[NAMESPACE].active) {
      console.log('[LUMA_RUNTIME] Same-version lifecycle already active, scheduling reconcile');
      if (typeof window[NAMESPACE].scheduleReconcile === 'function') {
        window[NAMESPACE].scheduleReconcile('reinjection');
      }
      return;
    }

    // Different version or no existing lifecycle: deactivate old and create new
    if (window[NAMESPACE]) {
      console.log('[FORENSIC_INJECT] Deactivating previous instance (v' + existingVersion + ')');
      if (typeof window[NAMESPACE].deactivate === 'function') {
        window[NAMESPACE].deactivate();
      }
      console.log('[FORENSIC_INJECT] Previous lifecycle teardown complete');
    }

    window[NAMESPACE] = {
      activate: activate,
      deactivate: function () { teardown(); },
      scheduleReconcile: scheduleReconcile,
      version: LUMA_INJECT_VERSION,
      active: false,
      documentId: DOCUMENT_ID,
      currentUrl: null,
      currentAppId: null,
      reconcileCount: 0,
      observerActive: false,
      historyWrapped: false,
    };

    console.log('[FORENSIC_INJECT] Lifecycle assigned (v' + LUMA_INJECT_VERSION + ')');
  } catch (e) { console.error('[CEF_INJECT_ERROR] namespace:', e); }

  if (document.readyState === 'complete' || document.readyState === 'interactive') {
    activate();
  } else {
    document.addEventListener('DOMContentLoaded', activate);
  }
})();
