/* global document, window */
// This classic script runs before the module graph. It can show a local error
// when loading that graph fails before React's error boundary exists.
;(function () {
  var reported = false
  function failed(kind) {
    if (reported) return
    reported = true
    var root = document.getElementById('root')
    if (!root) {
      root = document.createElement('div')
      root.id = 'root'
      document.body.appendChild(root)
    }
    if (root && !root.hasChildNodes()) {
      var main = document.createElement('main')
      main.setAttribute('role', 'alert')
      main.style.cssText = 'padding:2rem;font:16px sans-serif;color:#202124;background:#fff'
      var heading = document.createElement('h1')
      heading.textContent = 'Juniper could not start its interface'
      var detail = document.createElement('p')
      detail.textContent =
        'Your stored data was not reset. Restart Juniper and report the startup stage if this continues.'
      main.appendChild(heading)
      main.appendChild(detail)
      root.appendChild(main)
    }
    var native = window.__TAURI_INTERNALS__
    if (native && typeof native.invoke === 'function') {
      native.invoke('frontend_fatal', { report: 'bootstrap ' + kind }).catch(function () {})
    }
  }
  window.addEventListener('error', function () {
    failed('script error')
  })
  window.addEventListener('unhandledrejection', function () {
    failed('unhandled rejection')
  })
  window.__juniperBootstrapFailed = failed
})()
