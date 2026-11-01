// Vendored from esm.sh (see assets/vendor/codemirror/README.md) so the editor
// never depends on a third-party CDN being reachable.
import { EditorView, lineNumbers, drawSelection, highlightActiveLine,
         keymap } from './vendor/codemirror/_codemirror__view_6.43.8__es2022__view.mjs';
import { EditorState } from './vendor/codemirror/_codemirror__state_6.7.1__es2022__state.mjs';
import { defaultKeymap, history, historyKeymap, indentWithTab } from './vendor/codemirror/_codemirror__commands_6.10.4__es2022__commands.mjs';
import { markdown } from './vendor/codemirror/_codemirror__lang-markdown_6.5.2__es2022__lang-markdown.mjs';
import { oneDark } from './vendor/codemirror/_codemirror__theme-one-dark_6.1.3__es2022__theme-one-dark.mjs';
import { autocompletion } from './vendor/codemirror/_codemirror__autocomplete_6.20.3__es2022__autocomplete.mjs';
// Same generated WASM bindings module `assets/index.html` already imports
// `init` from — reuses the already-instantiated module rather than
// creating a second one (see backlog #91's design doc, "Editing side —
// detail"). By the time this file is dynamically imported (triggered from
// inside the running app, via tidl_load_editor), that module's init() has
// already resolved.
import { filter_mentions_json } from '/pkg/tidl.js';

const _editors = new Map();
let _nextId = 1;

function isDark() {
  return document.documentElement.classList.contains('dark');
}

// Unicode-aware, not \w: contact display names (and the query typed after
// "@") may contain non-ASCII letters (e.g. "José") — render_note_mentions
// already promises Unicode-case-insensitive matching, and this trigger
// must not regress that for the editing side.
const MENTION_TRIGGER = /@[\p{L}\p{N}_]*$/u;

function makeMentionSource(rec) {
  return function (context) {
    const match = context.matchBefore(MENTION_TRIGGER);
    if (!match && !context.explicit) return null;
    const query = (match ? match.text : '').replace(/^@/, '');
    let contacts;
    try {
      contacts = JSON.parse(filter_mentions_json(rec.contactsJson, query));
    } catch (e) {
      contacts = [];
    }
    return {
      from: match ? match.from : context.pos,
      options: contacts.map((c) => ({
        label: '@' + c.display_name,
        detail: c.org || undefined,
        apply: (view, _completion, from, to) => {
          view.dispatch({ changes: { from, to, insert: '@' + c.display_name + ' ' } });
        },
      })),
      filter: false, // ordering is already decided by filter_mentions_json
    };
  };
}

// `#` task/await chooser (backlog #129), mirroring the note editor's
// `util::find_active_hash`: offer Task…/Await… while the word after `#` is a
// prefix of a keyword, at a word boundary, before any `/` or whitespace.
const TODO_TRIGGER = /#([\p{L}]*)$/u;
const TODO_KEYS = ['task', 'await', 'todo', 'pending'];

function makeTodoSource(rec) {
  return function (context) {
    const match = context.matchBefore(TODO_TRIGGER);
    if (!match) return null;
    const partial = match.text.slice(1).toLowerCase();
    if (partial.includes('/')) return null;
    if (!TODO_KEYS.some((k) => k.startsWith(partial))) return null;

    const before = context.state.sliceDoc(0, match.from);
    const prev = before.slice(-1);
    if (before.length > 0 && !/[\s(\[{'"']/.test(prev)) return null;

    const removeTyped = (view, from, to) => {
      view.dispatch({ changes: { from, to, insert: '' } });
    };
    return {
      from: match.from,
      options: [
        {
          label: '#task',
          detail: 'New task…',
          apply: (view, _c, from, to) => {
            removeTyped(view, from, to);
            rec.onMenu && rec.onMenu('task');
          },
        },
        {
          label: '#await',
          detail: 'New await…',
          apply: (view, _c, from, to) => {
            removeTyped(view, from, to);
            rec.onMenu && rec.onMenu('await');
          },
        },
      ],
      filter: false,
    };
  };
}

window.__tidlEditorInit = function (el, content) {
  const id = _nextId++;
  const rec = { contactsJson: '[]', view: null, onTagHit: null, onMenu: null };
  const extensions = [
    lineNumbers(),
    drawSelection(),
    highlightActiveLine(),
    history(),
    keymap.of([...defaultKeymap, ...historyKeymap, indentWithTab]),
    markdown(),
    EditorView.lineWrapping,
    autocompletion({ override: [makeMentionSource(rec), makeTodoSource(rec)] }),
  ];
  if (isDark()) extensions.push(oneDark);

  const view = new EditorView({
    state: EditorState.create({ doc: content, extensions }),
    parent: el,
  });
  rec.view = view;
  // A click inside an existing tag (backlog #129) hands the document + caret to
  // Rust, which opens that tag's edit dialog. Rust returns nothing;
  // detection/boundary rules stay in one place (`markdown_tokens`). Uses
  // `click` (not `mousedown`) so the dialog appears after the pointer is
  // released — otherwise the same press's `mouseup`/`click` would land on the
  // freshly-rendered backdrop and immediately dismiss it.
  view.dom.addEventListener('click', (e) => {
    if (!rec.onTagHit) return;
    // Ignore clicks on a completion tooltip (e.g. picking a mention) — they're
    // not a click into the document text.
    if (e.target instanceof Element && e.target.closest('.cm-tooltip')) return;
    const pos = view.posAtCoords({ x: e.clientX, y: e.clientY });
    if (pos == null) return;
    rec.onTagHit(view.state.doc.toString(), pos);
  });
  _editors.set(id, rec);
  return id;
};

window.__tidlEditorGetContent = function (id) {
  const rec = _editors.get(id);
  return rec ? rec.view.state.doc.toString() : '';
};

window.__tidlEditorSetContacts = function (id, json) {
  const rec = _editors.get(id);
  if (rec) rec.contactsJson = json;
};

window.__tidlEditorSetHandlers = function (id, onTagHit, onMenu) {
  const rec = _editors.get(id);
  if (rec) {
    rec.onTagHit = onTagHit;
    rec.onMenu = onMenu;
  }
};

// Inserts `text` at the current caret, leaving the caret just after it. Used by
// the `#` create dialog.
window.__tidlEditorInsert = function (id, text) {
  const rec = _editors.get(id);
  if (!rec) return;
  const view = rec.view;
  const sel = view.state.selection.main;
  view.dispatch({
    changes: { from: sel.from, to: sel.to, insert: text },
    selection: { anchor: sel.from + text.length },
  });
  view.focus();
};

// Replaces the `[from, to)` code-unit range with `text` and puts the caret just
// after it. Used by the tag edit dialog; `from`/`to` come from Rust as UTF-16
// offsets.
window.__tidlEditorReplace = function (id, from, to, text) {
  const rec = _editors.get(id);
  if (!rec) return;
  const view = rec.view;
  const len = view.state.doc.length;
  const f = Math.max(0, Math.min(from, len));
  const t = Math.max(f, Math.min(to, len));
  view.dispatch({
    changes: { from: f, to: t, insert: text },
    selection: { anchor: f + text.length },
  });
  view.focus();
};

window.__tidlEditorDestroy = function (id) {
  const rec = _editors.get(id);
  if (rec) {
    rec.view.destroy();
    _editors.delete(id);
  }
};
