// Tiny, self-contained (no imports) loader that defers the CodeMirror editor
// bundle to first use instead of blocking the WASM app's boot on it — see
// assets/vendor/codemirror/README.md.
let _editorPromise = null;

window.__tidlLoadEditor = function() {
  if (!_editorPromise) {
    _editorPromise = import('./editor.js');
  }
  return _editorPromise;
};
