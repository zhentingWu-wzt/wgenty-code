/**
 * Token injection preload script for the wgenty-code Tauri desktop shell.
 *
 * This file is embedded into the Tauri binary at compile time (via
 * `include_str!`) and registered as a webview initialization script — it runs
 * before the web/ React app boots. It monkey-patches `window.fetch` so every
 * `/api/*` request gets an `Authorization: Bearer <token>` header injected by
 * the host, mirroring what the Vite dev proxy does for the browser frontend.
 *
 * The web/ source therefore needs zero changes: DaemonClient's
 * `fetch("/api/v1/...")` keeps working unchanged.
 *
 * The `__WGENTY_TOKEN__` placeholder is replaced by the Rust host at load time
 * with the eagerly-read token (a quoted JS string literal, or `null` if the
 * daemon isn't running yet).
 *
 * Security: the token is read from ~/.wgenty-code/daemon.token by the Rust host
 * process and only applied to loopback /api/* requests. The daemon binds
 * 127.0.0.1 only. This matches the Vite proxy's security model.
 */
(function () {
  if (window.__wgentyTokenPatched) return;
  window.__wgentyTokenPatched = true;

  // Eagerly-known token (embedded by host at load time). null if the daemon
  // wasn't running when the window opened.
  var eagerToken = __WGENTY_TOKEN__;
  var originalFetch = window.fetch.bind(window);

  function isApiRequest(input) {
    var url = typeof input === "string" ? input : (input && input.url) || "";
    return url.charAt(0) === "/" && url.slice(0, 5) === "/api/";
  }

  window.fetch = function (input, init) {
    init = init || {};
    if (isApiRequest(input)) {
      var token = eagerToken;
      if (token) {
        var headers = new Headers(init.headers || {});
        if (!headers.has("Authorization")) {
          headers.set("Authorization", "Bearer " + token);
        }
        init.headers = headers;
        init.credentials = "omit";
      }
    }
    return originalFetch(input, init).then(function (res) {
      // On 401, the daemon likely restarted and rotated the token. Refresh
      // from the host and retry once — avoids a window reload.
      if (res.status !== 401 || !isApiRequest(input)) return res;
      if (init.__wgentyRetried) return res;
      return window.__TAURI__.core
        .invoke("read_daemon_token")
        .then(function (fresh) {
          if (!fresh || fresh === eagerToken) return res;
          eagerToken = fresh;
          var headers2 = new Headers(init.headers || {});
          headers2.set("Authorization", "Bearer " + fresh);
          var retryInit = Object.assign({}, init, {
            headers: headers2,
            __wgentyRetried: true,
          });
          return originalFetch(input, retryInit);
        })
        .catch(function () {
          return res;
        });
    });
  };
})();
