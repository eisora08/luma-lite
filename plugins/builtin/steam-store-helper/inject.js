(function () {
  'use strict';

  var LUMA_INJECT_VERSION = '1.0.0';
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
    '.game_header_image_full'
  ];

  var _observer = null;
  var _historyPatched = false;
  var _pushState = null;
  var _replaceState = null;
  var _reconcileTimer = null;
  var _activeModal = null;
  var _lumaButton = null;
  var _activeAppId = null;
  var _originalPushState = window.history.pushState;
  var _originalReplaceState = window.history.replaceState;
  var _downloadUrl = '/api/download-package';
  var _isActive = false;

  function bridgeUrl(path) {
    return BRIDGE_SCHEME + '://' + BRIDGE_HOST + ':' + BRIDGE_PORT + path;
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

  function bridgeFetch(url, opts, label) {
    var method = (opts && opts.method) || 'GET';
    console.log(
      '[LUMA_BRIDGE]',
      'METHOD=' + method,
      'URL=' + url,
      'ORIGIN=' + window.location.origin,
      'PAGE=' + window.location.href
    );
    return fetch(url, opts)
      .then(function (res) {
        console.log(
          '[LUMA_BRIDGE]',
          'STATUS=' + res.status,
          'URL=' + url
        );
        return res;
      })
      .catch(function (err) {
        console.log(
          '[LUMA_BRIDGE]',
          'ERROR_NAME=' + err.name,
          'ERROR_MESSAGE=' + err.message,
          'ERROR_STACK=' + err.stack
        );
        throw err;
      });
  }

  function extractAppId() {
    var m = APP_URL_RE.exec(window.location.pathname);
    if (!m) {
      return null;
    }
    var id = m[1];
    if (!/^\d+$/.test(id)) {
      return null;
    }
    var num = parseInt(id, 10);
    if (num === 0) {
      return null;
    }
    if (id.length > MAX_ID_LENGTH) {
      return null;
    }
    return id;
  }

  function findActionContainer() {
    for (var i = 0; i < ACTION_SELECTORS.length; i++) {
      var el = document.querySelector(ACTION_SELECTORS[i]);
      if (el) {
        return el;
      }
    }
    return null;
  }

  function ensureKeyframes() {
    if (document.getElementById('luma_ssh_kf')) {
      return;
    }
    var style = document.createElement('style');
    style.id = 'luma_ssh_kf';
    style.textContent =
      '@keyframes luma_ssh_spin { 0% { transform: rotate(0deg); } 100% { transform: rotate(360deg); } } ' +
      '@keyframes luma_ssh_fade { 0% { opacity: 0; } 100% { opacity: 1; } } ' +
      '@keyframes luma_ssh_slide { 0% { transform: translateY(-10px); opacity: 0; } 100% { transform: translateY(0); opacity: 1; } }';
    document.head.appendChild(style);
  }

  function ensureStyles() {
    if (document.querySelector('style[data-lumaforge-style]')) {
      return;
    }
    var style = document.createElement('style');
    style.setAttribute('data-lumaforge-style', '');
    style.textContent =
      'button[data-lumaforge-extension="' + EXTENSION_ID + '"] {' +
      '  display: inline-flex; align-items: center; gap: 6px;' +
      '  padding: 8px 16px; border: none; border-radius: 4px;' +
      '  cursor: pointer; font-size: 13px; font-weight: 600;' +
      '  font-family: inherit; line-height: 1; white-space: nowrap;' +
      '  transition: opacity 0.15s, background 0.15s, box-shadow 0.15s;' +
      '  outline: none;' +
      '  background: linear-gradient(135deg, #5a2d8a, #7b3fb5);' +
      '  color: #fff;' +
      '  box-shadow: 0 1px 4px rgba(0,0,0,0.3);' +
      '}' +
      'button[data-lumaforge-extension="' + EXTENSION_ID + '"]:hover {' +
      '  background: linear-gradient(135deg, #6e36a4, #8f4ecc);' +
      '  box-shadow: 0 2px 8px rgba(90,45,138,0.5);' +
      '}' +
      'button[data-lumaforge-extension="' + EXTENSION_ID + '"]:active {' +
      '  transform: scale(0.97);' +
      '}' +
      'button[data-lumaforge-extension="' + EXTENSION_ID + '"]:disabled {' +
      '  opacity: 0.5; cursor: not-allowed; transform: none;' +
      '}' +
      'button[data-lumaforge-extension="' + EXTENSION_ID + '"][data-lumaforge-state="success"] {' +
      '  background: linear-gradient(135deg, #2d8a4e, #3fb56a);' +
      '}' +
      'button[data-lumaforge-extension="' + EXTENSION_ID + '"][data-lumaforge-state="error"] {' +
      '  background: linear-gradient(135deg, #8a2d2d, #b53f3f);' +
      '}' +
      'button[data-lumaforge-extension="' + EXTENSION_ID + '"] svg {' +
      '  width: 16px; height: 16px; fill: currentColor;' +
      '}' +
      'button[data-lumaforge-extension="' + EXTENSION_ID + '"] svg.luma-spinner {' +
      '  animation: luma_ssh_spin 0.8s linear infinite;' +
      '}' +
      '[data-lumaforge-modal="' + EXTENSION_ID + '"] {' +
      '  all: initial; font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;' +
      '}' +
      '[data-lumaforge-modal="' + EXTENSION_ID + '"] * {' +
      '  box-sizing: border-box;' +
      '}';
    document.head.appendChild(style);
  }

  function removeStyles() {
    var styleEl = document.querySelector('style[data-lumaforge-style]');
    if (styleEl && styleEl.parentNode) {
      styleEl.parentNode.removeChild(styleEl);
    }
  }

  function svgDownload(w, h) {
    return '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="' + (w || 16) + '" height="' + (h || 16) + '"><path d="M19 9h-4V3H9v6H5l7 7 7-7zM5 18v2h14v-2H5z"/></svg>';
  }

  function svgCloudDownload() {
    return '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="20" height="20"><path d="M19.35 10.04C18.67 6.59 15.64 4 12 4 9.11 4 6.6 5.64 5.35 8.04 2.34 8.36 0 10.91 0 14c0 3.31 2.69 6 6 6h13c2.76 0 5-2.24 5-5 0-2.64-2.05-4.78-4.65-4.96zM17 13l-5 5-5-5h3V9h4v4h3z"/></svg>';
  }

  function svgSpinner() {
    return '<svg class="luma-spinner" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="16" height="16"><path d="M12 4V1L8 5l4 4V6c3.31 0 6 2.69 6 6 0 1.01-.25 1.97-.7 2.8l1.46 1.46C19.54 15.03 20 13.57 20 12c0-4.42-3.58-8-8-8zm0 14c-3.31 0-6-2.69-6-6 0-1.01.25-1.97.7-2.8L5.24 7.74C4.46 8.97 4 10.43 4 12c0 4.42 3.58 8 8 8v3l4-4-4-4v3z" fill="#fff"/></svg>';
  }

  function svgCheck(w, h) {
    return '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="' + (w || 16) + '" height="' + (h || 16) + '"><path d="M9 16.17L4.83 12l-1.42 1.41L9 19 21 7l-1.41-1.41z"/></svg>';
  }

  function svgX() {
    return '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="18" height="18"><path d="M19 6.41L17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z"/></svg>';
  }

  function dot(hue) {
    var colorMap = {
      green: '#4caf50',
      blue: '#2196f3',
      red: '#f44336',
      gray: '#9e9e9e'
    };
    var c = colorMap[hue] || '#9e9e9e';
    return '<span style="display:inline-block;width:8px;height:8px;border-radius:50%;background:' + c + ';margin-right:4px;vertical-align:middle;"></span>';
  }

  function makeButton(appId) {
    var btn = document.createElement('button');
    btn.id = BTN_ID;
    btn.type = 'button';
    btn.setAttribute(BTN_MARKER_ATTR, BTN_MARKER_VAL);
    btn.setAttribute('data-lumaforge-control', 'download-package');
    btn.setAttribute(BTN_APPID_ATTR, appId);
    btn.setAttribute('aria-label', 'Download package via LumaForge for app ' + appId);
    btn.title = 'Download via LumaForge';
    btn.innerHTML = svgDownload() + ' ADD VIA LUMAFORGE';
    return btn;
  }

  function setButtonPending(btn) {
    if (!btn) return;
    btn.disabled = true;
    btn.setAttribute('data-lumaforge-pending', 'true');
    btn.innerHTML = svgSpinner() + ' ADDING...';
  }

  function setButtonSuccess(btn) {
    if (!btn) return;
    btn.disabled = false;
    btn.removeAttribute('data-lumaforge-pending');
    btn.setAttribute('data-lumaforge-state', 'success');
    btn.innerHTML = svgCheck() + ' ADDED';
  }

  function setButtonError(btn) {
    if (!btn) return;
    btn.disabled = false;
    btn.removeAttribute('data-lumaforge-pending');
    btn.setAttribute('data-lumaforge-state', 'error');
    btn.innerHTML = svgDownload() + ' ERROR - RETRY';
  }

  function setButtonIdle(btn, appId) {
    if (!btn) return;
    btn.disabled = false;
    btn.removeAttribute('data-lumaforge-pending');
    btn.removeAttribute('data-lumaforge-state');
    btn.setAttribute(BTN_APPID_ATTR, appId);
    btn.innerHTML = svgDownload() + ' ADD VIA LUMAFORGE';
  }

  function checkLocalStatus(appId, cb) {
    var controller;
    if (typeof AbortController !== 'undefined') {
      controller = new AbortController();
    }
    var url = localStatusUrl(appId);
    var opts = { method: 'GET', mode: 'cors' };
    if (controller) {
      opts.signal = controller.signal;
    }

    var timedOut = false;
    var timer = setTimeout(function () {
      timedOut = true;
      if (controller) {
        controller.abort();
      }
      cb(new Error('Local status request timed out'), null);
    }, LOCAL_STATUS_TIMEOUT_MS);

    bridgeFetch(url, opts, 'local-status')
      .then(function (res) {
        if (timedOut) return;
        clearTimeout(timer);
        if (!res.ok) {
          throw new Error('HTTP ' + res.status);
        }
        return res.json();
      })
      .then(function (data) {
        if (timedOut) return;
        var inLibrary = !!(data.inLibrary || data.in_library);
        cb(null, { inLibrary: inLibrary });
      })
      .catch(function (err) {
        if (timedOut) return;
        clearTimeout(timer);
        cb(err, null);
      });
  }

  function ensureLumaButtonExists() {
    var appId = extractAppId();
    if (!appId) {
      removeLumaButton();
      return null;
    }

    if (_lumaButton && _lumaButton.getAttribute(BTN_APPID_ATTR) === appId) {
      return _lumaButton;
    }

    removeLumaButton();

    var container = findActionContainer();
    if (!container) {
      return null;
    }

    var btn = makeButton(appId);
    btn.addEventListener('click', handleButtonClick);

    if (container.tagName === 'BUTTON' || (container.parentNode && container.parentNode.tagName === 'BUTTON')) {
      if (container.parentNode) {
        container.parentNode.parentNode.insertBefore(btn, container.parentNode.nextSibling);
      } else {
        container.parentNode.insertBefore(btn, container.nextSibling);
      }
    } else {
      container.parentNode.insertBefore(btn, container.nextSibling);
    }

    _lumaButton = btn;
    _activeAppId = appId;

    checkLocalStatus(appId, function (err, data) {
      if (err || !_isActive) {
        return;
      }
      if (data && data.inLibrary) {
        setButtonSuccess(_lumaButton);
      }
    });

    return btn;
  }

  function removeLumaButton() {
    if (_lumaButton && _lumaButton.parentNode) {
      _lumaButton.parentNode.removeChild(_lumaButton);
    }
    _lumaButton = null;
    _activeAppId = null;
  }

  function handleButtonClick(e) {
    e.stopPropagation();
    if (!_lumaButton) return;
    var appId = _lumaButton.getAttribute(BTN_APPID_ATTR);
    if (!appId) return;

    setButtonPending(_lumaButton);

    var url = bridgeUrl(_downloadUrl + '/' + appId);
    bridgeFetch(url, { method: 'GET', mode: 'cors' }, 'download-package')
      .then(function (res) {
        return res.json();
      })
      .then(function (data) {
        if (data && data.status === 'accepted') {
          setButtonSuccess(_lumaButton);
        } else {
          setButtonError(_lumaButton);
        }
      })
      .catch(function () {
        setButtonError(_lumaButton);
      });
  }

  function closeModal() {
    if (_activeModal) {
      if (_activeModal.parentNode) {
        _activeModal.parentNode.removeChild(_activeModal);
      }
      _activeModal = null;
    }
    document.body.style.overflow = '';
  }

  function openSourceModal(appId) {
    var backdrop = document.createElement('div');
    backdrop.setAttribute(MODAL_MARKER_ATTR, MODAL_MARKER_VAL);
    backdrop.style.cssText =
      'position:fixed;top:0;left:0;width:100%;height:100%;z-index:99999;' +
      'background:rgba(0,0,0,0.6);display:flex;align-items:center;justify-content:center;' +
      'animation:luma_ssh_fade 0.2s ease;';

    var panel = document.createElement('div');
    panel.style.cssText =
      'background:#1b1b2f;border-radius:12px;width:560px;max-width:90vw;' +
      'max-height:80vh;display:flex;flex-direction:column;overflow:hidden;' +
      'box-shadow:0 16px 48px rgba(0,0,0,0.5);animation:luma_ssh_slide 0.25s ease;';

    var header = document.createElement('div');
    header.style.cssText =
      'display:flex;align-items:center;padding:16px 20px;border-bottom:1px solid rgba(255,255,255,0.08);';

    var headerIcon = document.createElement('span');
    headerIcon.style.cssText = 'margin-right:10px;display:flex;';
    headerIcon.innerHTML = svgCloudDownload();
    header.appendChild(headerIcon);

    var headerTitle = document.createElement('span');
    headerTitle.style.cssText =
      'color:#e0d6f0;font-size:16px;font-weight:600;flex:1;';
    headerTitle.textContent = 'Select Download Source';
    header.appendChild(headerTitle);

    var versionSpan = document.createElement('span');
    versionSpan.style.cssText =
      'color:#7a6f9e;font-size:11px;margin-right:12px;';
    versionSpan.textContent = 'Runtime: ' + LUMA_INJECT_VERSION;
    header.appendChild(versionSpan);

    var closeBtn = document.createElement('button');
    closeBtn.type = 'button';
    closeBtn.style.cssText =
      'background:none;border:none;cursor:pointer;color:#7a6f9e;padding:4px;' +
      'display:flex;align-items:center;border-radius:4px;';
    closeBtn.innerHTML = svgX();
    closeBtn.addEventListener('click', closeModal);
    header.appendChild(closeBtn);

    panel.appendChild(header);

    var body = document.createElement('div');
    body.style.cssText =
      'padding:20px;overflow-y:auto;flex:1;min-height:120px;';

    var loadingDiv = document.createElement('div');
    loadingDiv.style.cssText =
      'display:flex;flex-direction:column;align-items:center;justify-content:center;padding:40px 0;';
    var spinnerEl = document.createElement('span');
    spinnerEl.style.cssText = 'margin-bottom:12px;';
    spinnerEl.innerHTML = svgSpinner();
    loadingDiv.appendChild(spinnerEl);
    var loadingText = document.createElement('span');
    loadingText.style.cssText = 'color:#7a6f9e;font-size:14px;';
    loadingText.textContent = 'Loading providers...';
    loadingDiv.appendChild(loadingText);
    body.appendChild(loadingDiv);
    panel.appendChild(body);

    var footer = document.createElement('div');
    footer.style.cssText =
      'padding:12px 20px;border-top:1px solid rgba(255,255,255,0.08);' +
      'display:flex;justify-content:flex-end;';

    var cancelBtn = document.createElement('button');
    cancelBtn.type = 'button';
    cancelBtn.style.cssText =
      'background:rgba(255,255,255,0.08);border:none;border-radius:6px;' +
      'padding:8px 20px;color:#c0b8d6;font-size:13px;cursor:pointer;';
    cancelBtn.textContent = 'Cancel';
    cancelBtn.addEventListener('click', closeModal);
    footer.appendChild(cancelBtn);
    panel.appendChild(footer);

    backdrop.appendChild(panel);
    document.body.appendChild(backdrop);
    document.body.style.overflow = 'hidden';
    _activeModal = backdrop;

    backdrop.addEventListener('click', function (e) {
      if (e.target === backdrop) {
        closeModal();
      }
    });

    var fetchUrl = providersUrl();
    console.log('[PROVIDER_FETCH]', 'VERSION=' + LUMA_INJECT_VERSION, 'METHOD=GET', 'URL=' + fetchUrl, 'PAGE=' + window.location.href, 'ORIGIN=' + window.location.origin, 'PROTOCOL=' + window.location.protocol, 'SECURE_CONTEXT=' + (window.isSecureContext));

    bridgeFetch(fetchUrl, { method: 'GET', mode: 'cors' }, 'providers')
      .then(function (res) {
        console.log('[PROVIDER_FETCH]', 'RESPONSE_RECEIVED', 'STATUS=' + res.status, 'OK=' + res.ok, 'TYPE=' + (res.type || 'basic'), 'RESPONSE_URL=' + res.url);
        if (!res.ok) {
          throw new Error('HTTP ' + res.status);
        }
        return res.json();
      })
      .then(function (providers) {
        body.innerHTML = '';
        renderSources(body, providers, appId);
      })
      .catch(function (err) {
        console.log('[PROVIDER_FETCH]', 'REJECTED', 'ERROR_NAME=' + err.name, 'ERROR_MESSAGE=' + err.message, 'ERROR_STACK=' + err.stack);
        body.innerHTML = '';
        var errorWrap = document.createElement('div');
        errorWrap.style.cssText = 'text-align:center;padding:30px 0;';
        var errorMsg = document.createElement('div');
        errorMsg.style.cssText = 'color:#f44336;font-size:14px;margin-bottom:8px;';
        errorMsg.textContent = 'Failed to load providers: ' + (err.message || 'Unknown error');
        errorWrap.appendChild(errorMsg);
        var errorDetail = document.createElement('div');
        errorDetail.style.cssText = 'color:#6a5f8a;font-size:12px;margin-bottom:16px;word-break:break-all;';
        errorDetail.textContent = err.name + ': ' + err.message;
        errorWrap.appendChild(errorDetail);
        var retryBtn = document.createElement('button');
        retryBtn.type = 'button';
        retryBtn.textContent = 'Retry';
        retryBtn.style.cssText =
          'background:linear-gradient(135deg,#5a2d8a,#7b3fb5);border:none;' +
          'border-radius:6px;padding:8px 24px;color:#fff;font-size:13px;cursor:pointer;';
        retryBtn.addEventListener('click', function () {
          closeModal();
          openSourceModal(appId);
        });
        errorWrap.appendChild(retryBtn);
        body.appendChild(errorWrap);
      });
  }

  function renderSources(body, sources, appId) {
    if (!sources || sources.length === 0) {
      var emptyMsg = document.createElement('div');
      emptyMsg.style.cssText = 'text-align:center;padding:30px 0;color:#7a6f9e;font-size:14px;';
      emptyMsg.textContent = 'No download sources available.';
      body.appendChild(emptyMsg);
      return;
    }
    for (var i = 0; i < sources.length; i++) {
      var source = sources[i];
      (function (src) {
        var card = document.createElement('div');
        card.style.cssText =
          'display:flex;align-items:center;padding:12px 16px;margin-bottom:8px;' +
          'background:rgba(255,255,255,0.04);border-radius:8px;cursor:pointer;' +
          'transition:background 0.15s;';
        card.addEventListener('mouseenter', function () {
          card.style.background = 'rgba(255,255,255,0.08)';
        });
        card.addEventListener('mouseleave', function () {
          card.style.background = 'rgba(255,255,255,0.04)';
        });

        var icon = document.createElement('span');
        icon.style.cssText = 'margin-right:12px;display:flex;color:#7b3fb5;';
        icon.innerHTML = svgDownload(20, 20);
        card.appendChild(icon);

        var info = document.createElement('div');
        info.style.cssText = 'flex:1;';

        var nameEl = document.createElement('div');
        nameEl.style.cssText = 'color:#e0d6f0;font-size:14px;font-weight:500;margin-bottom:2px;';
        nameEl.textContent = src.name || src.id || 'Unknown';
        info.appendChild(nameEl);

        var detailEl = document.createElement('div');
        detailEl.style.cssText = 'color:#6a5f8a;font-size:12px;';
        detailEl.textContent = src.description || src.detail || '';
        info.appendChild(detailEl);

        card.appendChild(info);

        var badge = document.createElement('span');
        if (src.adapterAvailable !== false) {
          badge.style.cssText =
            'background:rgba(76,175,80,0.15);color:#4caf50;font-size:11px;' +
            'font-weight:600;padding:2px 8px;border-radius:4px;';
          badge.textContent = 'Ready';
        } else {
          badge.style.cssText =
            'background:rgba(158,158,158,0.15);color:#9e9e9e;font-size:11px;' +
            'font-weight:600;padding:2px 8px;border-radius:4px;';
          badge.textContent = 'No Adapter';
        }
        card.appendChild(badge);

        card.addEventListener('click', function () {
          handleSourceClick(card, appId, src.id);
        });

        body.appendChild(card);
      })(source);
    }
  }

  function handleSourceClick(card, appId, sourceId) {
    card.style.opacity = '0.5';
    card.style.pointerEvents = 'none';
    var url = downloadUrl();
    bridgeFetch(url, {
      method: 'POST',
      mode: 'cors',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ appId: appId, sourceId: sourceId })
    }, 'download')
      .then(function (res) { return res.json(); })
      .then(function (data) {
        if (data && data.status === 'accepted') {
          card.style.opacity = '1';
          card.style.pointerEvents = '';
          card.innerHTML = '';
          card.style.background = 'rgba(76,175,80,0.1)';
          card.style.border = '1px solid rgba(76,175,80,0.3)';
          card.style.borderRadius = '8px';
          var successDiv = document.createElement('div');
          successDiv.style.cssText = 'display:flex;align-items:center;gap:8px;color:#4caf50;font-size:14px;font-weight:500;';
          successDiv.innerHTML = svgCheck(20, 20) + ' Download started';
          card.appendChild(successDiv);
        } else {
          card.style.opacity = '1';
          card.style.pointerEvents = '';
        }
      })
      .catch(function () {
        card.style.opacity = '1';
        card.style.pointerEvents = '';
      });
  }

  function handleDelegatedClick(e) {
    var target = e.target;
    if (target && target.id === BTN_ID) {
      e.stopPropagation();
      var appId = target.getAttribute(BTN_APPID_ATTR);
      if (appId) {
        openSourceModal(appId);
      }
    }
  }

  function scheduleReconcile() {
    if (_reconcileTimer) {
      clearTimeout(_reconcileTimer);
    }
    _reconcileTimer = setTimeout(function () {
      _reconcileTimer = null;
      if (!_isActive) return;
      if (!extractAppId()) {
        removeLumaButton();
        return;
      }
      ensureLumaButtonExists();
    }, RECONCILE_DEBOUNCE_MS);
  }

  function onUrlChange(trigger) {
    if (trigger) {
      console.log('[LUMA_SSH] UrlChange triggered by', trigger);
    }
    scheduleReconcile();
  }

  function patchHistory() {
    if (_historyPatched) {
      return;
    }

    window.history.pushState = function () {
      var args = arguments;
      var ret = _originalPushState.apply(this, args);
      onUrlChange('pushState');
      return ret;
    };

    window.history.replaceState = function () {
      var args = arguments;
      var ret = _originalReplaceState.apply(this, args);
      onUrlChange('replaceState');
      return ret;
    };

    window.addEventListener('popstate', function () {
      onUrlChange('popstate');
    });

    _historyPatched = true;
  }

  function restoreHistory() {
    if (!_historyPatched) {
      return;
    }
    window.history.pushState = _originalPushState;
    window.history.replaceState = _originalReplaceState;
    _historyPatched = false;
  }

  function startObserver() {
    if (_observer) {
      return;
    }
    _observer = new MutationObserver(function () {
      onUrlChange('mutation');
    });
    _observer.observe(document.body, { childList: true, subtree: true });
  }

  function stopObserver() {
    if (_observer) {
      _observer.disconnect();
      _observer = null;
    }
  }

  function activate() {
    if (_isActive) {
      ensureLumaButtonExists();
      return;
    }
    _isActive = true;
    ensureKeyframes();
    ensureStyles();
    patchHistory();
    startObserver();

    document.body.addEventListener('click', handleDelegatedClick);

    if (isSupportedPage()) {
      ensureLumaButtonExists();
    }
  }

  function teardown() {
    if (!_isActive) return;
    _isActive = false;
    if (_reconcileTimer) {
      clearTimeout(_reconcileTimer);
      _reconcileTimer = null;
    }
    stopObserver();
    restoreHistory();
    closeModal();
    removeLumaButton();
    removeStyles();

    if (document.body) {
      document.body.removeEventListener('click', handleDelegatedClick);
    }
  }

  function isLoginPage() {
    return /^\/(login|signin)/.test(window.location.pathname);
  }

  function isSupportedPage() {
    if (isLoginPage()) {
      return false;
    }
    return !!extractAppId();
  }

  var ns = {
    activate: activate,
    deactivate: teardown,
    version: LUMA_INJECT_VERSION
  };

  var prevNs = window[NAMESPACE];
  window[NAMESPACE] = ns;
  if (prevNs && prevNs !== ns) {
    console.log('[FORENSIC_INJECT] Previous lifecycle detected, deactivating old instance');
    if (typeof prevNs.deactivate === 'function') {
      try { prevNs.deactivate(); } catch (e) {}
    }
  }

  console.log(
    '[FORENSIC_INJECT]',
    'VERSION=' + LUMA_INJECT_VERSION,
    'URL=' + window.location.href,
    'ORIGIN=' + window.location.origin,
    'STATE=' + (isSupportedPage() ? 'supported' : 'unsupported')
  );

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', activate);
  } else {
    activate();
  }
})();
