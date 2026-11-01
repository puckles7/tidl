"""
Regression suite for the Contacts/Notes card-grid drag/resize/visibility
behavior (backlog #102 follow-up).

Unlike an earlier round of *interactive* Playwright exploration (dumping
positions via browser_evaluate for a human to eyeball, never saved as a
test), every assertion here is a real, coded `assert` against either the
persisted server state (`list_contacts`/`list_notes`, for anything that's
supposed to survive a reload) or the rendered DOM (`data-grid-row`/
`data-grid-col`/style attributes, for client-only preview state that's
never persisted -- Auto-layout preview, resize reflow).

Each test gets its own throwaway `tidl-server` instance (own port, own temp
data dir, admin user "alice"/"test-password") so cards seeded by one test can
never interfere with another test's cluster-detection math -- unlike the
shared session-scoped `tidl_server` other suites use.

Run via:
    DISABLE_SERVER_FN_HASH=1 cargo build -p tidl-server
    cargo leptos build
    pytest tests/test_cards_layout.py -v

The Playwright browser binaries are provided by the Nix dev shell via
PLAYWRIGHT_BROWSERS_PATH -- no `playwright install` step is needed.
"""

import socket
import time

import pytest
import requests
from playwright.sync_api import Page

from fixtures import TidlServer


# ── Server fixture (one throwaway instance per test) ─────────────────────────

def _free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


@pytest.fixture()
def cards_server(tidl_binary):
    srv = TidlServer(tidl_binary, port=_free_port(), username="alice", password="test-password")
    srv.start()
    yield srv
    srv.stop()


def _playwright_cookies(session: requests.Session) -> list[dict]:
    # `requests`' cookiejar appends Python cookiejar's ".local" suffix to a
    # bare hostname like "localhost" for its own domain-matching purposes
    # (RFC 2965 quirk) -- the browser's actual Host header is plain
    # "localhost", so a cookie stored under "localhost.local" never matches
    # and silently never gets sent, landing on the login page instead.
    return [
        {
            "name": c.name,
            "value": c.value,
            "domain": (c.domain or "").removesuffix(".local") or "localhost",
            "path": c.path or "/",
        }
        for c in session.cookies
    ]


@pytest.fixture()
def cards_page(browser, cards_server):
    """Logged-in page for `cards_server`, authenticated via cookie
    injection (skips the login form entirely -- this suite isn't testing
    login, and it's the WASM-hydration/console-message race the slower
    UI-driven login in `test_hydration.py` exists to handle)."""
    ctx = browser.new_context()
    ctx.add_cookies(_playwright_cookies(cards_server.login_session()))
    page = ctx.new_page()
    page.set_viewport_size({"width": 1400, "height": 1000})
    yield page
    ctx.close()


# ── Seeding helpers (via server fns, not the UI -- see module docstring) ─────

def make_contact_collection(server: TidlServer, name: str) -> int:
    return server.api_post_json(
        "create_contact_collection",
        {"input": {"name": name, "color": "#3b82f6", "shared": False, "members": []}},
    )


def make_contact(server: TidlServer, collection_id: int, display_name: str) -> int:
    dto = server.api_post_json(
        "create_contact",
        {
            "input": {
                "display_name": display_name,
                "given_name": None,
                "family_name": None,
                "middle_name": None,
                "name_prefix": None,
                "name_suffix": None,
                "nickname": None,
                "org": None,
                "title": None,
                "birthday": None,
                "anniversary": None,
                "notes": None,
                "categories": [],
                "emails": [],
                "phones": [],
                "urls": [],
                "impps": [],
                "addresses": [],
                "collection_id": collection_id,
            }
        },
    )
    return dto["id"]


def move_contacts_to(server: TidlServer, positions: list[dict]) -> None:
    """`positions`: list of {"id", "grid_row", "grid_col", "grid_row_span"}."""
    server.api_post_json("move_contacts", {"moves": positions})


def list_contacts(server: TidlServer) -> list[dict]:
    r = server.login_session().post(f"{server.url}/api/list_contacts", data={}, timeout=10)
    assert r.status_code == 200, f"list_contacts: {r.status_code} {r.text}"
    return r.json()


def make_note_collection(server: TidlServer, name: str) -> int:
    return server.api_post_json(
        "create_note_collection",
        {"input": {"name": name, "color": "#3b82f6", "shared": False, "members": []}},
    )


def make_note(server: TidlServer, collection_id: int, text: str, pinned: bool = False) -> int:
    return server.api_post_json(
        "create_note",
        {
            "input": {
                "collection_id": collection_id,
                "title": None,
                "text": text,
                "pinned": pinned,
                "kind": None,
                "priority": None,
                "calendar_name": None,
                "due_date": None,
            }
        },
    )


def move_notes_to(server: TidlServer, positions: list[dict]) -> None:
    server.api_post_json("move_notes", {"moves": positions})


def list_notes(server: TidlServer) -> list[dict]:
    r = server.login_session().post(f"{server.url}/api/list_notes", data={}, timeout=10)
    assert r.status_code == 200, f"list_notes: {r.status_code} {r.text}"
    return r.json()


def pos(id_: int, row: int, col: int, span: int = 15) -> dict:
    # 15 matches a bare contact card's real measured height (name + one
    # collection pill, no photo/org/etc) -- seeding anything else gives the
    # height self-correction pass (backlog #48) something to "fix" on the
    # very first render, firing an extra fire-and-forget `move_contacts`
    # whose network round trip has no ordering guarantee relative to a
    # later action's own persist (a genuine, pre-existing race orthogonal
    # to this suite's actual assertions -- see backlog #103). Seeding the
    # already-correct span means there is nothing to correct, so that
    # extra request never fires at all.
    return {"id": id_, "grid_row": row, "grid_col": col, "grid_row_span": span}


# ── DOM read helpers ──────────────────────────────────────────────────────────

def card_positions(page: Page, selector: str) -> dict[int, tuple[int, int]]:
    """{card id: (grid_row, grid_col)} for every currently-rendered card
    matching `selector` (e.g. "[data-contact-id]")."""
    id_attr = "data-contact-id" if "contact" in selector else "data-note-id"
    raw = page.eval_on_selector_all(
        selector,
        f"""els => els.map(el => ({{
            id: parseInt(el.getAttribute('{id_attr}')),
            row: parseInt(el.getAttribute('data-grid-row')),
            col: parseInt(el.getAttribute('data-grid-col')),
        }}))""",
    )
    return {e["id"]: (e["row"], e["col"]) for e in raw}


def card_rects(page: Page, selector: str) -> dict[int, dict]:
    id_attr = "data-contact-id" if "contact" in selector else "data-note-id"
    raw = page.eval_on_selector_all(
        selector,
        f"""els => els.map(el => {{
            const r = el.getBoundingClientRect();
            return {{ id: parseInt(el.getAttribute('{id_attr}')), x: r.x, y: r.y, w: r.width, h: r.height }};
        }})""",
    )
    return {e["id"]: e for e in raw}


def assert_no_overlaps(page: Page, selector: str) -> None:
    rects = card_rects(page, selector)
    ids = list(rects.keys())
    for i in range(len(ids)):
        for j in range(i + 1, len(ids)):
            a, b = rects[ids[i]], rects[ids[j]]
            overlap_x = a["x"] < b["x"] + b["w"] and b["x"] < a["x"] + a["w"]
            overlap_y = a["y"] < b["y"] + b["h"] and b["y"] < a["y"] + a["h"]
            assert not (overlap_x and overlap_y), (
                f"cards {ids[i]} and {ids[j]} overlap on screen: {a} vs {b}"
            )


def drag_card(page: Page, from_selector: str, to_x: float, to_y: float, api_path: str) -> None:
    """Drag the card at `from_selector` to screen point `(to_x, to_y)`,
    waiting for the resulting `api_path` (move_contacts/move_notes) round
    trip before returning."""
    box = page.eval_on_selector(
        from_selector, "el => { const r = el.getBoundingClientRect(); return {x: r.x + r.width/2, y: r.y + r.height/2}; }"
    )
    page.mouse.move(box["x"], box["y"])
    page.mouse.down()
    page.mouse.move(box["x"] + 15, box["y"], steps=3)
    page.mouse.move(to_x, to_y, steps=10)
    page.wait_for_timeout(150)
    with page.expect_response(lambda r: api_path in r.url, timeout=5_000):
        page.mouse.up()
    page.wait_for_timeout(200)


def toggle_collection_visibility(page: Page, name: str) -> None:
    # `.fs-collapse-btn`'s collapse/expand mechanism is mobile-only (its
    # sibling `.fs-content--collapsed` rule only applies inside a narrow-
    # viewport media query) -- at this suite's desktop viewport sizes the
    # sidebar content is already visible without clicking anything.
    #
    # `force=True`: the checkbox is the real toggle's underlying input, but
    # a sibling `.sidebar-pill-thumb`/`.sidebar-pill-track` span (the
    # visible switch UI) sits on top of it and intercepts pointer events --
    # exactly what a real click on the visible switch does, so bypassing
    # Playwright's actionability check here matches real user interaction,
    # not a workaround for something broken.
    page.click(f"input[aria-label='Show {name}']", force=True)
    page.wait_for_timeout(200)


# ── 1. Drag placement / displacement (assertion #1, #7) ──────────────────────

def test_drag_relocates_displaced_card_near_its_own_original_spot(cards_server, cards_page):
    coll = make_contact_collection(cards_server, "Team")
    a = make_contact(cards_server, coll, "Ada")
    b = make_contact(cards_server, coll, "Grace")
    c = make_contact(cards_server, coll, "Alan")
    # Real columns spread apart, not adjacent (0, 1, 3, ...): Manual mode's
    # real space is always `CARD_GRID_COLS`-wide and only ever compacted
    # down for display via `compact_cell`'s proportional `group_of` scaling
    # (backlog #106 fifth follow-up) -- two real columns just 1 apart can
    # legitimately compact into the *same* rendered column at a narrower
    # view width, same as two Auto-mode real columns always could. A real
    # test scenario needs real columns far enough apart to render as
    # distinct cells at this test's own view width, not adjacent integers.
    move_contacts_to(cards_server, [pos(a, 0, 0), pos(b, 0, 3), pos(c, 40, 6)])

    page = cards_page
    page.goto(f"{cards_server.url}/contacts", wait_until="load")
    page.wait_for_selector("[data-contact-id]", timeout=15_000)
    page.set_viewport_size({"width": 2200, "height": 1000})
    page.wait_for_timeout(300)

    b_before = card_positions(page, "[data-contact-id]")[b]
    b_rect = card_rects(page, "[data-contact-id]")[b]
    # Aim near B's top edge, not its pixel center -- `cell_at` resolves the
    # drop point's own pixel row directly (row-unit granularity is 8px), so
    # dropping mid-card would land partway through B's span, not on B's own
    # anchor cell.
    to_x, to_y = b_rect["x"] + b_rect["w"] / 2, b_rect["y"] + 4
    drag_card(page, f"[data-contact-id='{a}']", to_x, to_y, "move_contacts")

    # Rendered (render-space) positions, not the raw persisted DB row/col --
    # Manual mode now persists real, `CARD_GRID_COLS`-wide positions,
    # decoupled from whatever column count was active at drop time
    # (backlog #106 fifth follow-up), so the *raw* DB column is no longer
    # expected to equal the on-screen column the card was actually dropped
    # into. What "lands in the target column" actually means is what's on
    # screen right now, at this render width -- specifically, B's own
    # rendered column from just before the drag, since that's exactly
    # where A was aimed.
    after = card_positions(page, "[data-contact-id]")
    a_new = after[a]
    assert a_new[1] == b_before[1], (
        f"dragged card must land in the target column, landed at {a_new}, target was {b_before}"
    )
    assert abs(a_new[0] - 0) <= 6, f"dragged card must land near the target row, landed at {a_new}"
    b_new = after[b]
    assert b_new != a_new, "displaced card must not stay under the dropped card"
    dist = abs(b_new[0] - 0) + abs(b_new[1] - b_before[1])
    assert dist <= 20, f"displaced card should resettle near its own original spot, landed at {b_new}"
    # An isolated single-member cluster always starts at column 0 of its
    # own shelf on any repack (`pack_clusters_for_view`'s own placement
    # rule -- it never preserves an arbitrary absolute column across a
    # repack, only relative cluster membership and reading order), so C's
    # own column isn't expected to survive bit-for-bit here; its row (far
    # from the drag's own cluster) is what "untouched by someone else's
    # drop" actually means.
    assert after[c][0] == 40, "an uninvolved card must be untouched by someone else's drop"
    assert_no_overlaps(page, "[data-contact-id]")


def test_many_cards_never_overlap_across_resizes(cards_server, cards_page):
    coll = make_contact_collection(cards_server, "Big Team")
    names = [f"Person {i}" for i in range(12)]
    ids = [make_contact(cards_server, coll, n) for n in names]
    move_contacts_to(cards_server, [pos(i, (n % 4) * 25, n % 6) for n, i in enumerate(ids)])

    page = cards_page
    page.goto(f"{cards_server.url}/contacts", wait_until="load")
    page.wait_for_selector("[data-contact-id]", timeout=15_000)
    assert_no_overlaps(page, "[data-contact-id]")

    for width in (2200, 1400, 900, 1700):
        page.set_viewport_size({"width": width, "height": 1000})
        page.wait_for_timeout(300)
        assert_no_overlaps(page, "[data-contact-id]")


# ── 2. Resize reflow preserves gaggle shape (assertion #2) ───────────────────

def test_resize_down_preserves_gaggle_shape_and_grows_vertically(cards_server, cards_page):
    coll = make_contact_collection(cards_server, "Cluster")
    a = make_contact(cards_server, coll, "Ada")
    b = make_contact(cards_server, coll, "Grace")
    c = make_contact(cards_server, coll, "Alan")
    # A tight 3-card cluster (all within CLUSTER_ROW_THRESHOLD of each other).
    move_contacts_to(cards_server, [pos(a, 0, 0), pos(b, 0, 1), pos(c, 10, 2)])

    page = cards_page
    page.goto(f"{cards_server.url}/contacts", wait_until="load")
    page.wait_for_selector("[data-contact-id]", timeout=15_000)

    heights = []
    for width in (2200, 1400, 900):
        page.set_viewport_size({"width": width, "height": 1000})
        page.wait_for_timeout(300)
        pp = card_positions(page, "[data-contact-id]")
        assert {pp[a][1] for _ in [0]} == {pp[a][1]}  # no-op sanity, keeps linters quiet
        cols_used = {pp[a][1], pp[b][1], pp[c][1]}
        rows_used = [pp[a][0], pp[b][0], pp[c][0]]
        assert len(cols_used) >= 1
        heights.append(max(rows_used) - min(rows_used))
        assert_no_overlaps(page, "[data-contact-id]")

    assert heights[-1] >= heights[0], (
        f"narrower viewports must use at least as much vertical space for the same cluster: {heights}"
    )


def test_resize_round_trip_restores_original_layout(cards_server, cards_page):
    coll = make_contact_collection(cards_server, "RoundTrip")
    a = make_contact(cards_server, coll, "Ada")
    b = make_contact(cards_server, coll, "Grace")
    move_contacts_to(cards_server, [pos(a, 0, 0), pos(b, 20, 3)])

    page = cards_page
    page.goto(f"{cards_server.url}/contacts", wait_until="load")
    page.wait_for_selector("[data-contact-id]", timeout=15_000)
    page.set_viewport_size({"width": 1400, "height": 1000})
    page.wait_for_timeout(300)
    before = card_positions(page, "[data-contact-id]")

    page.set_viewport_size({"width": 900, "height": 1000})
    page.wait_for_timeout(300)
    page.set_viewport_size({"width": 1400, "height": 1000})
    page.wait_for_timeout(300)
    after = card_positions(page, "[data-contact-id]")

    assert after == before, f"resize round-trip must restore the original layout: {before} -> {after}"


# ── 3. Auto-layout is temporary (assertion #3) ────────────────────────────────

def test_auto_layout_off_restores_manual_positions_exactly(cards_server, cards_page):
    coll = make_contact_collection(cards_server, "AutoLayout")
    a = make_contact(cards_server, coll, "Zeta")
    b = make_contact(cards_server, coll, "Alpha")
    move_contacts_to(cards_server, [pos(a, 0, 0), pos(b, 50, 5)])

    page = cards_page
    page.goto(f"{cards_server.url}/contacts", wait_until="load")
    page.wait_for_selector("[data-contact-id]", timeout=15_000)
    before = card_positions(page, "[data-contact-id]")

    page.click("button:has-text('Auto-layout')")
    page.wait_for_timeout(300)
    during = card_positions(page, "[data-contact-id]")
    assert during != before, "Auto-layout preview should visibly repack (alphabetically) here"

    page.click("button:has-text('Auto-layout')")
    page.wait_for_timeout(300)
    after = card_positions(page, "[data-contact-id]")
    assert after == before, "turning Auto-layout back off must restore the exact manual positions"

    # And nothing was ever persisted while it was on.
    stored = {c["id"]: (c["grid_row"], c["grid_col"]) for c in list_contacts(cards_server)}
    assert stored[a] == (0, 0) and stored[b] == (50, 5)


def test_auto_layout_drag_only_reorders_stays_packed(cards_server, cards_page):
    coll = make_contact_collection(cards_server, "AutoDrag")
    names = ["Aardvark", "Bear", "Camel", "Dahlia"]
    ids = [make_contact(cards_server, coll, n) for n in names]

    page = cards_page
    page.goto(f"{cards_server.url}/contacts", wait_until="load")
    page.wait_for_selector("[data-contact-id]", timeout=15_000)
    page.click("button:has-text('Auto-layout')")
    page.wait_for_timeout(300)

    rects_before = card_rects(page, "[data-contact-id]")
    # Drag the alphabetically-last card onto the first one's position.
    last_id = ids[-1]
    first_rect = rects_before[ids[0]]
    to_x, to_y = first_rect["x"] + first_rect["w"] / 2, first_rect["y"] + first_rect["h"] / 2
    drag_card(page, f"[data-contact-id='{last_id}']", to_x, to_y, "reorder_contact")

    assert_no_overlaps(page, "[data-contact-id]")

    # "Packed" == no gap in *grid* space between consecutive cards in the
    # same real column: each next card's row starts exactly where the
    # previous one's span ends. Checked via the stored grid_row/grid_col/
    # grid_row_span attributes, not rendered pixel rects -- a card's
    # rendered pixel height can be smaller than its reserved
    # `grid_row_span` (an unrelated, pre-existing cosmetic slack in the
    # content-based span *estimate*, orthogonal to what this assertion is
    # actually about: whether the drag itself introduces a *logical* gap).
    entries = page.eval_on_selector_all(
        "[data-contact-id]",
        """els => els.map(el => ({
            id: parseInt(el.getAttribute('data-contact-id')),
            row: parseInt(el.getAttribute('data-grid-row')),
            col: parseInt(el.getAttribute('data-grid-col')),
            span: parseInt(el.getAttribute('data-grid-row-span')),
        }))""",
    )
    by_real_col: dict[int, list[dict]] = {}
    for e in entries:
        by_real_col.setdefault(e["col"], []).append(e)
    for col, cards in by_real_col.items():
        cards.sort(key=lambda e: e["row"])
        for prev, cur in zip(cards, cards[1:]):
            assert cur["row"] == prev["row"] + prev["span"], (
                f"Auto-mode drag must only reorder, never leave a gap -- "
                f"real column {col} has card {prev['id']} at row {prev['row']}"
                f"+{prev['span']} then card {cur['id']} at row {cur['row']}"
            )


# ── 4. Hiding a collection must not move visible cards (assertion #4) ────────

def test_hiding_a_collection_does_not_move_visible_cards(cards_server, cards_page):
    visible_coll = make_contact_collection(cards_server, "Visible Team")
    other_coll = make_contact_collection(cards_server, "Other Team")
    a = make_contact(cards_server, visible_coll, "Ada")
    b = make_contact(cards_server, visible_coll, "Grace")
    h = make_contact(cards_server, other_coll, "Hidden Person")
    # Position the soon-to-be-hidden card so it clusters with the visible
    # pair -- exactly the scenario backlog #102 was diagnosed from.
    move_contacts_to(cards_server, [pos(a, 0, 0), pos(h, 5, 1), pos(b, 10, 2)])

    page = cards_page
    page.goto(f"{cards_server.url}/contacts", wait_until="load")
    page.wait_for_selector("[data-contact-id]", timeout=15_000)
    before = card_positions(page, "[data-contact-id]")
    assert a in before and b in before and h in before

    toggle_collection_visibility(page, "Other Team")
    page.wait_for_timeout(300)
    after_hide = card_positions(page, "[data-contact-id]")
    assert h not in after_hide, "hidden card must not render"
    assert after_hide[a] == before[a], "visible card A must not move when an unrelated collection is hidden"
    assert after_hide[b] == before[b], "visible card B must not move when an unrelated collection is hidden"

    toggle_collection_visibility(page, "Other Team")
    page.wait_for_timeout(300)
    after_show = card_positions(page, "[data-contact-id]")
    assert after_show[a] == before[a]
    assert after_show[b] == before[b]
    assert after_show[h] == before[h], "re-shown card must be back at its original spot"


# ── 5. Dragging onto a hidden card's slot relocates it (assertion #5) ────────

def test_drag_onto_hidden_cards_slot_relocates_it_underwater(cards_server, cards_page):
    # `a` (the drag target) and `v` (the card being dragged) give the
    # pointer-resolution math a real, on-screen anchor to translate
    # against -- a lone visible card with no other visible neighbor falls
    # back to `expand_cell`'s pre-existing (and, for an empty destination
    # column, inherently ambiguous -- see its own doc comment) compaction
    # scheme, which is a real but different, already-documented backlog
    # #102 limitation, not what this assertion is testing. `h` is stored
    # at exactly `a`'s real cell -- an already-colliding starting state
    # that's invisible and harmless until "ToHide" is shown -- so dragging
    # `v` onto `a`'s own (visible, precisely targetable) rendered position
    # both displaces `a` *and* leaves `h` colliding with wherever the drop
    # actually lands, directly exercising the commit-time relocation.
    visible_coll = make_contact_collection(cards_server, "Visible")
    hidden_coll = make_contact_collection(cards_server, "ToHide")
    a = make_contact(cards_server, visible_coll, "Anchor")
    v = make_contact(cards_server, visible_coll, "Visible Person")
    h = make_contact(cards_server, hidden_coll, "Hidden Person")
    move_contacts_to(cards_server, [pos(a, 0, 0), pos(v, 40, 3), pos(h, 0, 0)])

    page = cards_page
    page.goto(f"{cards_server.url}/contacts", wait_until="load")
    page.wait_for_selector("[data-contact-id]", timeout=15_000)

    toggle_collection_visibility(page, "ToHide")
    page.wait_for_timeout(300)
    assert h not in card_positions(page, "[data-contact-id]")

    a_rect = card_rects(page, "[data-contact-id]")[a]
    target_x, target_y = a_rect["x"] + a_rect["w"] / 2, a_rect["y"] + 4
    drag_card(page, f"[data-contact-id='{v}']", target_x, target_y, "move_contacts")

    after = {c["id"]: c for c in list_contacts(cards_server)}
    v_new = (after[v]["grid_row"], after[v]["grid_col"])
    a_new = (after[a]["grid_row"], after[a]["grid_col"])
    assert v_new[1] == 0, f"dragged card must land in the anchor's own column, landed at {v_new}"
    assert abs(v_new[0] - 0) <= 6, f"dragged card must land near the anchor's own row, landed at {v_new}"
    assert a_new != v_new, "the displaced anchor must not stay under the dropped card"
    h_new = (after[h]["grid_row"], after[h]["grid_col"])
    assert h_new != (0, 0), "hidden card must relocate rather than silently collide with the drop"
    assert h_new != a_new, "hidden card must not collide with the displaced anchor either"

    toggle_collection_visibility(page, "ToHide")
    page.wait_for_timeout(300)
    # Not a DOM-position equality check against `h_new`: that's h's raw
    # *stored* (real) cell, while what renders here is its Manual-mode
    # cluster-packed *render* position (backlog #102) -- legitimately a
    # different coordinate space. The actual invariant re-showing must
    # uphold is just: no overlap, now that h is visible again.
    assert h in card_positions(page, "[data-contact-id]"), "re-shown hidden card must render"
    assert_no_overlaps(page, "[data-contact-id]")


# ── 6. Keep this layout matches the preview exactly (assertion #8) ──────────

def test_keep_this_layout_persists_exactly_what_was_previewed(cards_server, cards_page):
    visible_coll = make_contact_collection(cards_server, "KeepVisible")
    hidden_coll = make_contact_collection(cards_server, "KeepHidden")
    a = make_contact(cards_server, visible_coll, "Aaron")
    b = make_contact(cards_server, visible_coll, "Beatrice")
    h = make_contact(cards_server, hidden_coll, "HiddenOne")
    move_contacts_to(cards_server, [pos(a, 0, 0), pos(b, 30, 4), pos(h, 0, 0)])

    page = cards_page
    page.goto(f"{cards_server.url}/contacts", wait_until="load")
    page.wait_for_selector("[data-contact-id]", timeout=15_000)
    toggle_collection_visibility(page, "KeepHidden")
    page.wait_for_timeout(300)

    page.click("button:has-text('Auto-layout')")
    page.wait_for_timeout(800)

    with page.expect_response(lambda r: "move_contacts" in r.url, timeout=5_000):
        page.click("button:has-text('Keep this layout')")
    page.wait_for_timeout(300)
    # What's on screen the instant "Keep this layout" commits -- not the
    # raw tight-pack preview from just before clicking it. The two can
    # legitimately differ (backlog #106): the committed positions are run
    # through `pack_clusters_for_view` once before persisting, since that
    # function is only guaranteed idempotent on its own prior output, never
    # on an arbitrary masonry pack -- persisting the raw tight pack as-is
    # meant the very next reflow re-clustered it into a visibly different
    # arrangement than what had just been "kept".
    kept = card_positions(page, "[data-contact-id]")

    # Ground truth is a real reload (not the raw persisted DB row/col):
    # Manual mode now persists a canonical, `CARD_GRID_COLS`-wide abstract
    # encoding decoupled from whatever column count is actually on screen
    # (backlog #106 fourth follow-up), so the raw DB fields are no longer
    # expected to equal the *rendered* cells "kept" reads -- only a fresh
    # render of the same persisted data, at the same view width, is.
    # `hidden_collections` is a client-only signal that resets on reload,
    # so "KeepHidden" is re-hidden before comparing, to restore the same
    # filtered view rather than let its reappearance shift cells for an
    # entirely different reason than what this assertion is about.
    page.goto(f"{cards_server.url}/contacts", wait_until="load")
    page.wait_for_selector("[data-contact-id]", timeout=15_000)
    toggle_collection_visibility(page, "KeepHidden")
    page.wait_for_timeout(300)
    after_reload = card_positions(page, "[data-contact-id]")
    assert after_reload[a] == kept[a], "Keep this layout must persist exactly what it committed"
    assert after_reload[b] == kept[b], "Keep this layout must persist exactly what it committed"

    persisted = {c["id"]: (c["grid_row"], c["grid_col"]) for c in list_contacts(cards_server)}
    assert persisted[h] != (0, 0) or persisted[h] != persisted[a], (
        "the hidden contact must not end up colliding with the freshly persisted layout"
    )


# ── 6b. Keep this layout, direct preview-vs-commit comparison, every card
# (backlog #106 follow-up, reported live 2026-09-28: "Keep this layout" no
# longer keeps the layout at all, and now also overlaps cards) ──────────────

def test_keep_this_layout_matches_the_observed_auto_layout_preview_for_every_card(
    cards_server, cards_page
):
    # Three collections, deliberately scattered (not already auto-layout-
    # packed, and not one-per-column) so committing the preview actually
    # has to move things -- an already-tidy seed wouldn't exercise the
    # commit path at all.
    coll_a = make_contact_collection(cards_server, "PreviewMatchA")
    coll_b = make_contact_collection(cards_server, "PreviewMatchB")
    coll_c = make_contact_collection(cards_server, "PreviewMatchC")
    a_ids = [make_contact(cards_server, coll_a, f"A{i}") for i in range(3)]
    b_ids = [make_contact(cards_server, coll_b, f"B{i}") for i in range(2)]
    c_ids = [make_contact(cards_server, coll_c, f"C{i}") for i in range(3)]
    all_ids = a_ids + b_ids + c_ids
    move_contacts_to(cards_server, [
        pos(a_ids[0], 40, 5), pos(a_ids[1], 5, 2), pos(a_ids[2], 70, 0),
        pos(b_ids[0], 15, 6), pos(b_ids[1], 90, 3),
        pos(c_ids[0], 0, 7), pos(c_ids[1], 55, 1), pos(c_ids[2], 20, 4),
    ])

    page = cards_page
    page.goto(f"{cards_server.url}/contacts", wait_until="load")
    page.wait_for_selector("[data-contact-id]", timeout=15_000)
    page.set_viewport_size({"width": 1400, "height": 1000})
    page.wait_for_timeout(300)

    page.click("button:has-text('Auto-layout')")
    page.wait_for_timeout(800)
    # Compared as rendered *pixel* rects, not `data-grid-row`/`-col`
    # attributes: during the Auto-layout preview those attributes carry
    # `pack_for_view`'s real (storage-space) columns (`ContactCard`'s
    # `render_position: None` branch), but after "Keep this layout" commits
    # they carry `cluster_cache`'s view-space columns instead
    # (`render_position: Some`) -- two different, legitimately non-equal
    # coordinate systems that both resolve to the same on-screen pixels via
    # `card_style`'s own `compact_cell` call either way. Pixels are the one
    # thing that must not move.
    previewed = card_rects(page, "[data-contact-id]")
    assert set(previewed) == set(all_ids), (
        "every contact must be visible during the Auto-layout preview"
    )

    with page.expect_response(lambda r: "move_contacts" in r.url, timeout=5_000):
        page.click("button:has-text('Keep this layout')")
    page.wait_for_timeout(300)
    kept = card_rects(page, "[data-contact-id]")

    for cid in all_ids:
        before, after = previewed[cid], kept.get(cid)
        assert after is not None and (after["x"], after["y"]) == (before["x"], before["y"]), (
            f"'Keep this layout' must commit exactly what was just previewed for "
            f"contact {cid}: previewed={before} kept={after}"
        )
    assert_no_overlaps(page, "[data-contact-id]")


def test_hiding_a_collection_still_works_after_a_keep_this_layout_commit(cards_server, cards_page):
    # Reported live 2026-09-28: after "Keep this layout" returns to manual
    # mode, a second Auto-layout click silently no-ops; navigating away and
    # back to the page fixes it. A bare second Auto-layout click can't
    # actually prove this either way, though: `pack_for_view`'s own repack
    # is a deterministic function of (ids, spans, sort order, column
    # count) -- with nothing else changed, a *correctly* working second
    # click reproduces the exact arrangement "Keep this layout" just
    # committed, same as a stuck one would. Hiding a collection right
    # after the commit is a clean, unambiguous probe instead: it depends
    # on the same `hidden_collections` signal that a stuck rebuild effect
    # would have silently dropped, and there's no legitimate reason for it
    # to reproduce the same result whether or not the effect reruns --
    # hidden cards must disappear, full stop.
    coll_a = make_contact_collection(cards_server, "ReAutoA")
    coll_b = make_contact_collection(cards_server, "ReAutoB")
    a_ids = [make_contact(cards_server, coll_a, f"A{i}") for i in range(3)]
    b_ids = [make_contact(cards_server, coll_b, f"B{i}") for i in range(2)]
    move_contacts_to(cards_server, [
        pos(a_ids[0], 40, 5), pos(a_ids[1], 5, 2), pos(a_ids[2], 70, 0),
        pos(b_ids[0], 15, 6), pos(b_ids[1], 90, 3),
    ])

    page = cards_page
    page.goto(f"{cards_server.url}/contacts", wait_until="load")
    page.wait_for_selector("[data-contact-id]", timeout=15_000)
    page.set_viewport_size({"width": 1400, "height": 1000})
    page.wait_for_timeout(300)

    page.click("button:has-text('Auto-layout')")
    page.wait_for_timeout(800)
    with page.expect_response(lambda r: "move_contacts" in r.url, timeout=5_000):
        page.click("button:has-text('Keep this layout')")
    page.wait_for_timeout(300)
    # "Keep this layout" must return to manual mode -- the toolbar must
    # show "Auto-layout" again, not still be in the preview state.
    assert page.locator("button:has-text('Auto-layout')").is_visible()
    assert page.locator("button:has-text('Keep this layout')").count() == 0

    toggle_collection_visibility(page, "ReAutoB")
    page.wait_for_timeout(300)
    visible = set(card_positions(page, "[data-contact-id]"))
    assert visible == set(a_ids), (
        f"hiding a collection right after a 'Keep this layout' commit must still work: "
        f"expected only {a_ids} visible, got {sorted(visible)}"
    )


def test_notes_hiding_a_collection_still_works_after_a_keep_this_layout_commit(
    cards_server, cards_page
):
    # Mirrors the identical contacts regression above -- `notes::NotesView`
    # has the same rebuild effect shape and the same fix.
    coll_a = make_note_collection(cards_server, "ReAutoA")
    coll_b = make_note_collection(cards_server, "ReAutoB")
    a_ids = [make_note(cards_server, coll_a, f"A{i}") for i in range(3)]
    b_ids = [make_note(cards_server, coll_b, f"B{i}") for i in range(2)]
    move_notes_to(cards_server, [
        pos(a_ids[0], 40, 5), pos(a_ids[1], 5, 2), pos(a_ids[2], 70, 0),
        pos(b_ids[0], 15, 6), pos(b_ids[1], 90, 3),
    ])

    page = cards_page
    page.goto(f"{cards_server.url}/notes", wait_until="load")
    page.wait_for_selector("[data-note-id]", timeout=15_000)
    page.set_viewport_size({"width": 1400, "height": 1000})
    page.wait_for_timeout(300)

    page.click("button:has-text('Auto-layout')")
    page.wait_for_timeout(800)
    with page.expect_response(lambda r: "move_notes" in r.url, timeout=5_000):
        page.click("button:has-text('Keep this layout')")
    page.wait_for_timeout(300)
    assert page.locator("button:has-text('Auto-layout')").is_visible()
    assert page.locator("button:has-text('Keep this layout')").count() == 0

    toggle_collection_visibility(page, "ReAutoB")
    page.wait_for_timeout(300)
    visible = set(card_positions(page, "[data-note-id]"))
    assert visible == set(a_ids), (
        f"hiding a collection right after a 'Keep this layout' commit must still work: "
        f"expected only {a_ids} visible, got {sorted(visible)}"
    )


# ── Notes: same drag mechanism, sanity pass (assertions #1/#7 on Notes) ──────

def test_notes_drag_relocates_displaced_note_near_its_own_original_spot(cards_server, cards_page):
    coll = make_note_collection(cards_server, "Notes Team")
    a = make_note(cards_server, coll, "Note A")
    b = make_note(cards_server, coll, "Note B")
    c = make_note(cards_server, coll, "Note C")
    # Real columns spread apart, not adjacent -- see the identical comment
    # in test_drag_relocates_displaced_card_near_its_own_original_spot.
    move_notes_to(cards_server, [pos(a, 0, 0), pos(b, 0, 3), pos(c, 40, 6)])

    page = cards_page
    page.goto(f"{cards_server.url}/notes", wait_until="load")
    page.wait_for_selector("[data-note-id]", timeout=15_000)
    page.set_viewport_size({"width": 2200, "height": 1000})
    page.wait_for_timeout(300)

    before = card_positions(page, "[data-note-id]")
    b_before, c_before = before[b], before[c]
    b_rect = card_rects(page, "[data-note-id]")[b]
    to_x, to_y = b_rect["x"] + b_rect["w"] / 2, b_rect["y"] + b_rect["h"] / 2
    drag_card(page, f"[data-note-id='{a}']", to_x, to_y, "move_notes")

    # Rendered (render-space) positions, not the raw persisted DB row/col --
    # see the identical comment in
    # test_drag_relocates_displaced_card_near_its_own_original_spot for why.
    after = card_positions(page, "[data-note-id]")
    assert after[a] == (0, b_before[1])
    # An uninvolved note's real position is completely untouched by
    # someone else's drop (backlog #106 fifth follow-up: a drag only ever
    # writes the abstract entries for the ids it actually touched), so its
    # *rendered* position -- a pure function of that unchanged real
    # position and the unchanged view width -- must be unchanged too.
    assert after[c] == c_before, "an uninvolved card must be untouched by someone else's drop"
    assert_no_overlaps(page, "[data-note-id]")


# ── 9. First-load must not collapse cards with a stale/placeholder span ──────

def test_notes_first_load_does_not_collapse_with_placeholder_spans(cards_server, cards_page):
    # Mirrors how `seed-demo` (and any pre-#48-migrated data) actually
    # seeds notes: `grid_row_span=1`, a placeholder nothing has corrected
    # yet, deliberately mismatched against each note's real rendered
    # height -- exactly the case that must trigger (and survive) the
    # client-side height self-correction pass on first render (backlog
    # #102 follow-up: the cluster-packing render-position map going stale
    # after a *successful* self-correction, since that persist never
    # bumps `refresh`, silently under-booked every subsequent note's
    # height and packed neighbors too close -- visually overlapping).
    coll = make_note_collection(cards_server, "Realistic")
    ids = [
        make_note(cards_server, coll, f"Note body #{i} with enough text to force a real multi-line card height")
        for i in range(6)
    ]
    move_notes_to(cards_server, [pos(i, n * 2, 0, span=1) for n, i in enumerate(ids)])

    page = cards_page
    page.goto(f"{cards_server.url}/notes", wait_until="load")
    page.wait_for_selector("[data-note-id]", timeout=15_000)
    # Give the RAF-scheduled self-correction pass time to run and settle.
    page.wait_for_timeout(1500)

    assert_no_overlaps(page, "[data-note-id]")


# ── 10. Auto-mode drag must never strand a sibling, leaving a gap ────────────

def test_auto_layout_drag_leaves_no_gap_in_the_source_column(cards_server, cards_page):
    # Backlog #102 follow-up, found via live manual testing: a card
    # dragged OUT of a column that had another card below it left that
    # sibling stranded at its stale pre-drop row -- nothing ever pulled it
    # up to close the gap, since the drop's own column-scoped cascade only
    # ever touches the *target* column, and a successful `reorder_note`
    # never bumps `refresh` to pick up a corrected full repack.
    coll = make_note_collection(cards_server, "GapCheck")
    names = [f"Note {i:02d}" for i in range(10)]
    ids = [make_note(cards_server, coll, n) for n in names]

    page = cards_page
    page.goto(f"{cards_server.url}/notes", wait_until="load")
    page.wait_for_selector("[data-note-id]", timeout=15_000)
    page.click("button:has-text('Auto-layout')")
    page.wait_for_timeout(500)

    # Find a real column that currently holds 2+ cards -- guaranteed with
    # 10 notes packed across a handful of columns at this viewport.
    entries = page.eval_on_selector_all(
        "[data-note-id]",
        """els => els.map(el => ({
            id: parseInt(el.getAttribute('data-note-id')),
            row: parseInt(el.getAttribute('data-grid-row')),
            col: parseInt(el.getAttribute('data-grid-col')),
        }))""",
    )
    by_col: dict[int, list[dict]] = {}
    for e in entries:
        by_col.setdefault(e["col"], []).append(e)
    target_col = next(col for col, es in by_col.items() if len(es) >= 2)
    top_card = min(by_col[target_col], key=lambda e: e["row"])

    # Drag that column's top card onto a different column entirely.
    other_col = next(col for col in by_col if col != target_col)
    dest_card = by_col[other_col][0]
    from_sel = f"[data-note-id='{top_card['id']}']"
    dest_rect = card_rects(page, "[data-note-id]")[dest_card["id"]]
    drag_card(page, from_sel, dest_rect["x"] + dest_rect["w"] / 2, dest_rect["y"] + 4, "reorder_note")

    assert_no_overlaps(page, "[data-note-id]")

    # "No gap" == every real column's cards form a contiguous run of rows
    # starting at 0, exactly like `test_auto_layout_drag_only_reorders_-
    # stays_packed`'s check, but specifically re-verifying the *source*
    # column the dragged card just vacated.
    after = page.eval_on_selector_all(
        "[data-note-id]",
        """els => els.map(el => ({
            id: parseInt(el.getAttribute('data-note-id')),
            row: parseInt(el.getAttribute('data-grid-row')),
            col: parseInt(el.getAttribute('data-grid-col')),
            span: parseInt(el.getAttribute('data-grid-row-span')),
        }))""",
    )
    by_real_col: dict[int, list[dict]] = {}
    for e in after:
        by_real_col.setdefault(e["col"], []).append(e)
    for col, cards in by_real_col.items():
        cards.sort(key=lambda e: e["row"])
        assert cards[0]["row"] == 0, (
            f"column {col} must start packed at row 0, found {cards[0]['row']} -- {cards}"
        )
        for prev, cur in zip(cards, cards[1:]):
            assert cur["row"] == prev["row"] + prev["span"], (
                f"column {col} has a gap: card {prev['id']} at row {prev['row']}+{prev['span']} "
                f"then card {cur['id']} at row {cur['row']}"
            )


# ── 11. Manual-mode drag shadow must track the pointer's real column the
# whole time, never snapping to an unrelated column (backlog #104) ──────────

def rendered_cols(page: Page, grid_selector: str = ".contacts-grid") -> int:
    return page.eval_on_selector(
        grid_selector,
        "el => getComputedStyle(el).gridTemplateColumns.split(' ').length",
    )


def sample_shadow_x_during_drag(
    page: Page, from_selector: str, to_x: float, to_y: float, steps: int = 10
) -> list[float | None]:
    """Picks up `from_selector`'s card and moves the pointer toward
    `(to_x, to_y)` in `steps` increments WITHOUT releasing, sampling the
    live `.grid-drop-shadow`'s on-screen x position after each move (`None`
    if no shadow is showing at that sample). The caller must release
    (`page.mouse.up()`) afterward -- this never does, so it can also be
    used mid-drag to inspect other DOM state."""
    box = page.eval_on_selector(
        from_selector,
        "el => { const r = el.getBoundingClientRect(); return {x: r.x + r.width/2, y: r.y + r.height/2}; }",
    )
    page.mouse.move(box["x"], box["y"])
    page.mouse.down()
    page.mouse.move(box["x"] + 15, box["y"], steps=3)  # cross the 5px drag threshold
    page.wait_for_timeout(50)
    xs: list[float | None] = []
    for i in range(1, steps + 1):
        x = box["x"] + (to_x - box["x"]) * i / steps
        y = box["y"] + (to_y - box["y"]) * i / steps
        page.mouse.move(x, y, steps=2)
        page.wait_for_timeout(30)
        shadow = page.query_selector(".grid-drop-shadow")
        xs.append(shadow.bounding_box()["x"] if shadow else None)
    return xs


def test_manual_drag_shadow_tracks_the_pointers_real_column(cards_server, cards_page):
    # Regression test for the bug reported live (2026-09-26): dragging a
    # card straight through wide-open empty space, directly above another
    # card in the very same column, made the drop-shadow visibly snap
    # sideways to an unrelated card in a different column simply because
    # that other card happened to be closer in row-distance to the
    # pointer's *current* position -- even though the pointer's own
    # screen column never changed for the whole drag. Root cause and fix
    # are in `grid::nearest_neighbor_translate` and
    # `DragItemState::last_render_cell` -- see backlog #104.
    coll = make_contact_collection(cards_server, "ShadowTrack")
    anchor = make_contact(cards_server, coll, "Anchor")  # alone, far above, column 0
    near = make_contact(cards_server, coll, "Near")  # different column, row-close to `dragged`
    dragged = make_contact(cards_server, coll, "Dragged")  # same column as `anchor`, far below
    move_contacts_to(
        cards_server,
        [pos(anchor, 0, 0), pos(near, 40, 1), pos(dragged, 40, 0)],
    )

    page = cards_page
    page.goto(f"{cards_server.url}/contacts", wait_until="load")
    page.wait_for_selector("[data-contact-id]", timeout=15_000)

    anchor_rect = card_rects(page, "[data-contact-id]")[anchor]
    target_x = anchor_rect["x"] + anchor_rect["w"] / 2
    target_y = anchor_rect["y"] + 4

    xs = sample_shadow_x_during_drag(page, f"[data-contact-id='{dragged}']", target_x, target_y)
    page.mouse.up()
    page.wait_for_timeout(200)

    assert all(x is not None for x in xs), f"drop-shadow must stay visible for the whole drag: {xs}"
    spread = max(xs) - min(xs)
    assert spread < 5, (
        f"drop-shadow's on-screen x drifted by {spread:.0f}px while dragging straight up "
        f"through a single column -- it must track the pointer's real column throughout, "
        f"not snap sideways toward a row-closer card in a different column: {xs}"
    )
    assert_no_overlaps(page, "[data-contact-id]")


# ── 12. Two independent gaggles ("islands") stay cohesive and never mix
# across a resize that crosses the "columns < 4" reshape threshold
# (backlog #104) ─────────────────────────────────────────────────────────────

def _row_extent(page: Page, contact_id: int) -> tuple[int, int]:
    """(grid_row, grid_row + grid_row_span) for one rendered contact card."""
    el = f"[data-contact-id='{contact_id}']"
    row = int(page.eval_on_selector(el, "e => e.getAttribute('data-grid-row')"))
    span = int(page.eval_on_selector(el, "e => e.getAttribute('data-grid-row-span')"))
    return row, row + span


def test_two_islands_stay_cohesive_and_separate_across_a_resize(cards_server, cards_page):
    coll_a = make_contact_collection(cards_server, "IslandA")
    coll_b = make_contact_collection(cards_server, "IslandB")
    a_ids = [make_contact(cards_server, coll_a, f"A{i}") for i in range(4)]
    b_ids = [make_contact(cards_server, coll_b, f"B{i}") for i in range(4)]

    # Seed island A as a 2x2 block top-left, island B offset right and
    # below -- matching the reported scenario -- via `move_contacts_to`
    # (deterministic), the same setup style every other test in this suite
    # uses. Real *pixel* drag targets can't reliably aim at an exact
    # abstract cell: a drop's resolved column lives in the full 8-real-
    # column abstract space, not clamped to whatever's currently in view,
    # so pixel-targeting eight separate drags against a narrower on-screen
    # width is inherently approximate. The one real drag below (moving a
    # single island-B card) is what actually exercises the drag path
    # `cluster_cache` (backlog #104) is meant to fix -- that a drop must
    # never disturb an unrelated gaggle -- which doesn't need pixel-exact
    # aim, just "pick this card up and drop it somewhere nearby".
    span = 15
    move_contacts_to(cards_server, [
        pos(a_ids[0], 0, 0, span), pos(a_ids[1], 0, 1, span),
        pos(a_ids[2], span, 0, span), pos(a_ids[3], span, 1, span),
        pos(b_ids[0], span, 3, span), pos(b_ids[1], span, 4, span),
        pos(b_ids[2], span * 2, 3, span), pos(b_ids[3], span * 2, 4, span),
    ])

    page = cards_page
    page.goto(f"{cards_server.url}/contacts", wait_until="load")
    page.wait_for_selector("[data-contact-id]", timeout=15_000)
    # 2200px measured (against this suite's 280px card / 16px gap / sidebar
    # layout) to render 5 real columns -- wide enough for both 2-wide
    # islands to sit side by side with the required inter-cluster gap
    # column between them. Set *after* the initial load (not as the page's
    # starting viewport) and given a moment to settle, mirroring every
    # other resize in this suite (e.g.
    # `test_resize_down_preserves_gaggle_shape_and_grows_vertically`) --
    # the column count is measured from the rendered container's own
    # width via a `resize`-event listener, which a pre-navigation initial
    # viewport doesn't reliably trigger a remeasure against before first
    # paint. If this fails on `rendered_cols`, the layout changed and
    # every width below needs re-measuring the same way.
    page.set_viewport_size({"width": 2200, "height": 1000})
    page.wait_for_timeout(300)
    assert rendered_cols(page) == 5, "calibration: this test assumes a 5-column view at 2200px"
    assert_no_overlaps(page, "[data-contact-id]")

    def island_row_extent(ids: list[int]) -> tuple[int, int]:
        extents = [_row_extent(page, cid) for cid in ids]
        return min(e[0] for e in extents), max(e[1] for e in extents)

    heights: dict[str, list[int]] = {"a": [], "b": []}
    snapshots: list[dict[int, tuple[int, int]]] = []
    # 2200 (5 cols, the setup width) -> 1450 (3 cols, crosses the "< 4"
    # reshape threshold a 4-member island hits) -> 1150 (2 cols, further
    # elongation) -> back to 2200 -- each measured the same way as above.
    #
    # No separate "islands never mix" adjacency check here anymore
    # (backlog #106 fifth follow-up): that used to assert `detect_clusters`'
    # own column-same-or-adjacent-and-row-close merge rule never fired
    # across the two islands, but Manual mode no longer runs any such
    # cluster-detection/repacking pass at all -- display is now a pure,
    # exact `compact_cell` projection of each card's own real position, so
    # there's no "merge" step left to break. `assert_no_overlaps` below
    # already covers the invariant that actually matters (no card visually
    # collides with another), and the exact-round-trip check after this
    # loop covers the invariant backlog #106 is actually about.
    for width in (2200, 1450, 1150, 2200):
        page.set_viewport_size({"width": width, "height": 1000})
        page.wait_for_timeout(300)
        assert_no_overlaps(page, "[data-contact-id]")

        a_lo, a_hi = island_row_extent(a_ids)
        b_lo, b_hi = island_row_extent(b_ids)
        heights["a"].append(a_hi - a_lo)
        heights["b"].append(b_hi - b_lo)
        snapshots.append(card_positions(page, "[data-contact-id]"))

    # Exact round trip (backlog #106 fifth follow-up): the loop above ends
    # back at 2200px, the same width it started at, with no drag in
    # between -- Manual mode's real positions were never touched by any of
    # these resizes, so the render must be byte-for-byte identical to the
    # very first 2200px snapshot, not just "roughly still two islands".
    assert snapshots[-1] == snapshots[0], (
        f"a resize round trip back to the original width must reproduce the exact same "
        f"layout: first={snapshots[0]} last={snapshots[-1]}"
    )

    # The core "elongates vertically" claim: once the view narrows past
    # the point where a 4-member island can no longer render as a single
    # flat row (columns < 4), each island's own vertical extent must grow
    # -- and the round trip back to the original width must restore it
    # exactly (mirrors `test_resize_round_trip_restores_original_layout`).
    for label in ("a", "b"):
        h = heights[label]
        assert h[1] >= h[0] and h[2] >= h[1], (
            f"island {label} must use at least as much vertical space as columns narrow "
            f"past the < 4 threshold: {h}"
        )
        assert h[3] == h[0], (
            f"island {label} must return to its original vertical extent once back at the "
            f"original column count: {h}"
        )

    # "Card dash placement... remains close to the mouse pointer position"
    # -- verified once more here, in this heavier multi-island scenario:
    # the drop-shadow must stay near the pointer throughout a drag, never
    # jumping away to a distant, unrelated cell. Runs at the narrowest (3-
    # column) width from the loop above, where the reshaped islands give
    # the nearest-neighbor heuristic the most competing anchors to choose
    # between.
    page.set_viewport_size({"width": 1450, "height": 1000})
    page.wait_for_timeout(300)
    start_rect = card_rects(page, "[data-contact-id]")[a_ids[0]]
    start_x = start_rect["x"] + start_rect["w"] / 2
    start_y = start_rect["y"] + start_rect["h"] / 2
    end_x, end_y = start_x + 30, start_y + 120

    page.mouse.move(start_x, start_y)
    page.mouse.down()
    page.mouse.move(start_x + 15, start_y, steps=3)
    page.wait_for_timeout(50)
    max_dist = 0.0
    steps = 10
    for i in range(1, steps + 1):
        x = start_x + (end_x - start_x) * i / steps
        y = start_y + (end_y - start_y) * i / steps
        page.mouse.move(x, y, steps=2)
        page.wait_for_timeout(30)
        shadow = page.query_selector(".grid-drop-shadow")
        assert shadow is not None, "drop-shadow must stay visible for the whole drag"
        box = shadow.bounding_box()
        cx, cy = box["x"] + box["width"] / 2, box["y"] + box["height"] / 2
        max_dist = max(max_dist, ((cx - x) ** 2 + (cy - y) ** 2) ** 0.5)
    page.mouse.up()
    page.wait_for_timeout(200)

    # Generous bound -- the shadow snaps to whole grid cells (a real card
    # width/height), not the exact pixel the pointer sits at, but it must
    # never be showing a *different, distant* cell.
    assert max_dist < start_rect["w"] + start_rect["h"], (
        f"drop-shadow center drifted {max_dist:.0f}px from the pointer -- it must stay near "
        f"the actual mouse position throughout the drag"
    )
    assert_no_overlaps(page, "[data-contact-id]")


# ── 13. A drop lands in the column (not the exact row) its shadow showed ────

def test_dropped_card_lands_exactly_where_its_shadow_showed(cards_server, cards_page):
    # Regression test for the bug reported live (2026-09-27): a Manual-mode
    # drop landed a card at a wildly unrelated *column*, not where the dashed
    # shadow had just shown moving to. Backlog #106 fixed the column/row
    # round-trip; backlog #107 later made `column_offsets` order colliding
    # real columns by their topmost occupied row, which can flip *which whole
    # block renders on top* after an edit changes a column's top card -- a
    # discontinuity #107 documents explicitly as an accepted trade-off. So
    # after a drop the dragged card is still guaranteed to land in the same
    # column its shadow showed (no sideways teleport), but its exact row can
    # shift when a block flip moves the whole stack. The invariants asserted
    # here: the drag's own column is honoured, and nothing ever overlaps.
    #
    # Six cards across three real columns, seeded at a viewport narrow
    # enough to compact them all into very few *virtual* columns: the more
    # real columns share one virtual column, the more a drag's own
    # occupancy change reshapes that virtual column's stacking order --
    # exactly the case #107's content ordering reflows.
    coll = make_contact_collection(cards_server, "ShadowMatch")
    ids = [make_contact(cards_server, coll, f"P{i}") for i in range(6)]
    move_contacts_to(cards_server, [
        pos(ids[0], 0, 0, 10), pos(ids[1], 0, 2, 30), pos(ids[2], 0, 4, 15),
        pos(ids[3], 40, 0, 20), pos(ids[4], 40, 2, 10), pos(ids[5], 40, 4, 25),
    ])

    page = cards_page
    page.goto(f"{cards_server.url}/contacts", wait_until="load")
    page.wait_for_selector("[data-contact-id]", timeout=15_000)
    page.set_viewport_size({"width": 900, "height": 1000})
    page.wait_for_timeout(300)
    cols = rendered_cols(page)
    assert cols <= 2, f"calibration: this test needs a heavily compacted view, got {cols} columns at 900px"
    assert_no_overlaps(page, "[data-contact-id]")

    # Drag one card a real, but otherwise arbitrary, distance -- no need to
    # aim at a specific abstract cell (a real pixel drag can't reliably do
    # that anyway); the assertion below must hold wherever it actually lands.
    dragged = ids[0]
    start = card_rects(page, "[data-contact-id]")[dragged]
    start_x, start_y = start["x"] + start["w"] / 2, start["y"] + start["h"] / 2
    end_x, end_y = start_x + 20, start_y + 250

    page.mouse.move(start_x, start_y)
    page.mouse.down()
    page.mouse.move(start_x + 10, start_y, steps=3)
    page.wait_for_timeout(50)
    for i in range(1, 11):
        x = start_x + (end_x - start_x) * i / 10
        y = start_y + (end_y - start_y) * i / 10
        page.mouse.move(x, y, steps=2)
        page.wait_for_timeout(30)

    shadow = page.query_selector(".grid-drop-shadow")
    assert shadow is not None, "drop-shadow must be visible right before release"
    shadow_box = shadow.bounding_box()

    page.mouse.up()
    page.wait_for_timeout(250)

    dropped = card_rects(page, "[data-contact-id]")[dragged]
    dx = abs(dropped["x"] - shadow_box["x"])
    assert dx < 3, (
        f"dropped card must land in the same column its own shadow last showed -- "
        f"shadow was at ({shadow_box['x']:.0f}, {shadow_box['y']:.0f}), "
        f"card landed at ({dropped['x']:.0f}, {dropped['y']:.0f})"
    )
    assert_no_overlaps(page, "[data-contact-id]")


# ── 14. Manual-mode drag must never move an unrelated, non-colliding card
# (backlog #106 follow-up, reported live 2026-09-27) ──────────────────────────

def test_dragging_one_note_never_moves_an_unrelated_far_away_note(cards_server, cards_page):
    # "Spooky action at a distance": dragging one note upward, in wide-open
    # space nowhere near any other card, moved a *completely unrelated*
    # note elsewhere on screen. Two different collections so the bug isn't
    # collection-scoped -- both are visible simultaneously, matching the
    # reported repro.
    coll_a = make_note_collection(cards_server, "Alone A")
    coll_b = make_note_collection(cards_server, "Alone B")
    a = make_note(cards_server, coll_a, "Note A")
    b = make_note(cards_server, coll_b, "Note B")
    # Far apart in both row and column -- draggng `a` a short distance
    # upward must have zero effect on `b`.
    move_notes_to(cards_server, [pos(a, 40, 2), pos(b, 0, 5)])

    page = cards_page
    page.goto(f"{cards_server.url}/notes", wait_until="load")
    page.wait_for_selector("[data-note-id]", timeout=15_000)
    page.set_viewport_size({"width": 1400, "height": 1000})
    page.wait_for_timeout(300)

    before_b = card_rects(page, "[data-note-id]")[b]

    a_rect = card_rects(page, "[data-note-id]")[a]
    to_x = a_rect["x"] + a_rect["w"] / 2
    to_y = a_rect["y"] - 60  # a short move straight up, still wide open space
    drag_card(page, f"[data-note-id='{a}']", to_x, to_y, "move_notes")

    after_b = card_rects(page, "[data-note-id]")[b]
    assert abs(after_b["x"] - before_b["x"]) < 2 and abs(after_b["y"] - before_b["y"]) < 2, (
        f"an unrelated note must never move when a different, far-away note is dragged: "
        f"before={before_b}, after={after_b}"
    )
    assert_no_overlaps(page, "[data-note-id]")


# ── 15. A card displaced during the live preview must land there, not
# somewhere else, once actually dropped (backlog #106 follow-up) ─────────────

def test_note_displaced_during_preview_lands_exactly_there_after_drop(cards_server, cards_page):
    # The dashed shadow (and any card it visibly pushes aside) is the
    # user's contract for what a drop will do. Dragging near another note
    # so it gets displaced during the live preview, then releasing, must
    # leave that displaced note exactly where the preview last showed it --
    # not somewhere else once the drop actually commits.
    coll = make_note_collection(cards_server, "PreviewMatch")
    dragged = make_note(cards_server, coll, "Dragged")
    neighbor = make_note(cards_server, coll, "Neighbor")
    # Real columns spread apart, not adjacent -- see the identical comment
    # in test_drag_relocates_displaced_card_near_its_own_original_spot.
    move_notes_to(cards_server, [pos(dragged, 0, 0), pos(neighbor, 0, 3)])

    page = cards_page
    page.goto(f"{cards_server.url}/notes", wait_until="load")
    page.wait_for_selector("[data-note-id]", timeout=15_000)
    page.set_viewport_size({"width": 2200, "height": 1000})
    page.wait_for_timeout(300)

    neighbor_before = card_rects(page, "[data-note-id]")[neighbor]
    start = card_rects(page, "[data-note-id]")[dragged]
    start_x, start_y = start["x"] + start["w"] / 2, start["y"] + start["h"] / 2
    # Land directly on top of `neighbor`'s own cell -- guaranteed to
    # displace it.
    to_x = neighbor_before["x"] + neighbor_before["w"] / 2
    to_y = neighbor_before["y"] + 4

    page.mouse.move(start_x, start_y)
    page.mouse.down()
    page.mouse.move(start_x + 10, start_y, steps=3)
    page.wait_for_timeout(50)
    page.mouse.move(to_x, to_y, steps=10)
    page.wait_for_timeout(200)

    neighbor_during = card_rects(page, "[data-note-id]")[neighbor]
    assert (abs(neighbor_during["x"] - neighbor_before["x"]) > 2
            or abs(neighbor_during["y"] - neighbor_before["y"]) > 2), (
        "calibration: dropping directly on the neighbor's cell must displace it during the "
        f"live preview -- before={neighbor_before}, during={neighbor_during}"
    )

    with page.expect_response(lambda r: "move_notes" in r.url, timeout=5_000):
        page.mouse.up()
    page.wait_for_timeout(200)

    neighbor_after = card_rects(page, "[data-note-id]")[neighbor]
    assert (abs(neighbor_after["x"] - neighbor_during["x"]) < 3
            and abs(neighbor_after["y"] - neighbor_during["y"]) < 3), (
        f"the displaced neighbor must land exactly where the live preview last showed it -- "
        f"preview was at ({neighbor_during['x']:.0f}, {neighbor_during['y']:.0f}), "
        f"landed at ({neighbor_after['x']:.0f}, {neighbor_after['y']:.0f})"
    )
    assert_no_overlaps(page, "[data-note-id]")


# ── 16. A drop must never fling a displaced note into another column
# (backlog #106 follow-up; row reflow within a column is #107's trade-off) ────

def test_drop_never_flings_a_displaced_note_far_away(cards_server, cards_page):
    coll = make_note_collection(cards_server, "NoFlying")
    dragged = make_note(cards_server, coll, "Dragged")
    neighbor = make_note(cards_server, coll, "Neighbor")
    others = [make_note(cards_server, coll, f"Other {i}") for i in range(4)]
    move_notes_to(cards_server, [
        pos(dragged, 0, 0), pos(neighbor, 0, 1),
        *[pos(oid, 40 + i * 20, i) for i, oid in enumerate(others)],
    ])

    page = cards_page
    page.goto(f"{cards_server.url}/notes", wait_until="load")
    page.wait_for_selector("[data-note-id]", timeout=15_000)
    page.set_viewport_size({"width": 1400, "height": 1000})
    page.wait_for_timeout(300)

    before = card_rects(page, "[data-note-id]")
    neighbor_before = before[neighbor]
    start = before[dragged]
    start_x, start_y = start["x"] + start["w"] / 2, start["y"] + start["h"] / 2
    to_x = neighbor_before["x"] + neighbor_before["w"] / 2
    to_y = neighbor_before["y"] + 4
    drag_card(page, f"[data-note-id='{dragged}']", to_x, to_y, "move_notes")

    after = card_rects(page, "[data-note-id]")
    # Backlog #107's content-based ordering of colliding real columns can
    # flip which whole block renders on top after an edit changes a column's
    # top card, so a local drop may reflow a block vertically -- that
    # discontinuity is an accepted trade-off, not the "spurious horizontal
    # fling" this test was written to guard against. What must still hold:
    # no card changes its view column, none is lost, and nothing overlaps.
    for nid in [dragged, neighbor, *others]:
        assert abs(after[nid]["x"] - before[nid]["x"]) < 2, (
            f"note {nid} must not change column from a single local drag -- "
            f"before={before[nid]}, after={after[nid]}"
        )
    assert_no_overlaps(page, "[data-note-id]")


# ── 17. A resize round-trip must restore the exact layout even after a real
# drag already happened (backlog #106 follow-up) ──────────────────────────────

def test_manual_layout_survives_a_resize_round_trip_after_real_drags(cards_server, cards_page):
    coll_a = make_note_collection(cards_server, "ResizeIslandA")
    coll_b = make_note_collection(cards_server, "ResizeIslandB")
    a_ids = [make_note(cards_server, coll_a, f"A{i}") for i in range(3)]
    b_ids = [make_note(cards_server, coll_b, f"B{i}") for i in range(3)]
    move_notes_to(cards_server, [
        pos(a_ids[0], 0, 0), pos(a_ids[1], 0, 1), pos(a_ids[2], 20, 0),
        pos(b_ids[0], 60, 3), pos(b_ids[1], 60, 4), pos(b_ids[2], 80, 3),
    ])

    page = cards_page
    page.goto(f"{cards_server.url}/notes", wait_until="load")
    page.wait_for_selector("[data-note-id]", timeout=15_000)
    page.set_viewport_size({"width": 1400, "height": 1000})
    page.wait_for_timeout(300)

    # One real drag to nudge island A -- exercising the actual drag-commit
    # path (not just seeded positions) before the resize round trip, since
    # that's what actually exposed the bug live.
    a0_rect = card_rects(page, "[data-note-id]")[a_ids[0]]
    to_x = a0_rect["x"] + a0_rect["w"] / 2 + 20
    to_y = a0_rect["y"] + a0_rect["h"] + 60
    drag_card(page, f"[data-note-id='{a_ids[0]}']", to_x, to_y, "move_notes")
    # Let the RAF-scheduled height self-correction pass fully settle
    # before taking any snapshot -- these short-text seed notes commonly
    # measure shorter than their placeholder span, and capturing a
    # snapshot mid-correction would race the very re-cluster this test is
    # about (mirrors `test_notes_first_load_does_not_collapse_with_placeholder_spans`).
    page.wait_for_timeout(1000)

    # A drag's own on-screen result is patched in directly, exact render-
    # space precision (so a drop lands exactly where its shadow showed,
    # backlog #106's original fix) -- it is *not* itself
    # `pack_clusters_for_view`'s own canonical output, so the very first
    # resize settles it into that canonical form (same contract as "Keep
    # this layout": see its own doc comment). One settling resize round
    # trip first, then `baseline` is captured *after* that -- this is
    # what a second, identical round trip actually has to reproduce
    # exactly.
    page.set_viewport_size({"width": 900, "height": 1000})
    page.wait_for_timeout(300)
    page.set_viewport_size({"width": 1400, "height": 1000})
    page.wait_for_timeout(300)
    baseline = card_positions(page, "[data-note-id]")

    page.set_viewport_size({"width": 900, "height": 1000})
    page.wait_for_timeout(300)
    page.set_viewport_size({"width": 1400, "height": 1000})
    page.wait_for_timeout(300)

    after = card_positions(page, "[data-note-id]")
    # Exact per-id position equality (backlog #106 fourth follow-up): the
    # on-screen render cache is now always a pure, deterministic function
    # of (the canonical abstract positions, the current view width),
    # recomputed fresh from scratch rather than composed on top of
    # whatever was last displayed -- a resize only ever reads the
    # (untouched) abstract state, never writes it. Round-tripping back to
    # the exact same view width must therefore reproduce the exact same
    # render, not merely a cohesion-preserving approximation of it.
    assert after == baseline, (
        f"a resize round trip back to the same width must reproduce the exact same layout: "
        f"baseline={baseline} after={after}"
    )
    assert_no_overlaps(page, "[data-note-id]")


# ── 18. "Keep this layout" must actually keep the layout, including across
# a full page reload (backlog #106 follow-up) ─────────────────────────────────

def test_keep_this_layout_persists_after_reload_with_prior_drags(cards_server, cards_page):
    coll_a = make_note_collection(cards_server, "KeepIslandA")
    coll_b = make_note_collection(cards_server, "KeepIslandB")
    a_ids = [make_note(cards_server, coll_a, f"KA{i}") for i in range(3)]
    b_ids = [make_note(cards_server, coll_b, f"KB{i}") for i in range(3)]
    move_notes_to(cards_server, [
        pos(a_ids[0], 0, 0), pos(a_ids[1], 0, 1), pos(a_ids[2], 20, 0),
        pos(b_ids[0], 60, 3), pos(b_ids[1], 60, 4), pos(b_ids[2], 80, 3),
    ])

    page = cards_page
    page.goto(f"{cards_server.url}/notes", wait_until="load")
    page.wait_for_selector("[data-note-id]", timeout=15_000)
    page.set_viewport_size({"width": 1400, "height": 1000})
    page.wait_for_timeout(300)

    # A real drag first, matching actual usage -- not just seeded data.
    a0_rect = card_rects(page, "[data-note-id]")[a_ids[0]]
    to_x = a0_rect["x"] + a0_rect["w"] / 2 + 20
    to_y = a0_rect["y"] + a0_rect["h"] + 60
    drag_card(page, f"[data-note-id='{a_ids[0]}']", to_x, to_y, "move_notes")

    page.click("button:has-text('Auto-layout')")
    page.wait_for_timeout(800)

    with page.expect_response(lambda r: "move_notes" in r.url, timeout=5_000):
        page.click("button:has-text('Keep this layout')")
    page.wait_for_timeout(500)
    # What's on screen the instant "Keep this layout" commits -- not the
    # raw tight-pack preview from just before clicking it. The two can
    # legitimately differ (backlog #106): the committed positions are run
    # through `pack_clusters_for_view` once before persisting, since that
    # function is only guaranteed idempotent on its own prior output, never
    # on an arbitrary masonry pack -- persisting the raw tight pack as-is
    # meant the very next reflow (a resize, or this same page reloaded
    # fresh) re-clustered it into a visibly different arrangement than what
    # had just been "kept". This is the actual contract: whatever commit
    # shows right now must be exactly what survives forever after.
    kept = card_positions(page, "[data-note-id]")

    # Ground truth is a real reload (not just the DOM already on screen) --
    # "Keep this layout" must survive a full refetch, not just look right
    # because nothing forced a re-cluster yet.
    page.goto(f"{cards_server.url}/notes", wait_until="load")
    page.wait_for_selector("[data-note-id]", timeout=15_000)
    page.set_viewport_size({"width": 1400, "height": 1000})
    page.wait_for_timeout(300)
    after_reload = card_positions(page, "[data-note-id]")

    assert after_reload == kept, (
        f"'Keep this layout' must persist exactly what it committed, and it must "
        f"survive a full page reload: kept={kept} after_reload={after_reload}"
    )
    assert_no_overlaps(page, "[data-note-id]")


# ── 19. Toggling Auto-layout on then off must restore the manual gaggles,
# not merge/scramble them (backlog #106 follow-up, reported live 2026-09-27) ──

def test_auto_layout_off_restores_manual_gaggles(cards_server, cards_page):
    coll_a = make_contact_collection(cards_server, "AutoOffIslandA")
    coll_b = make_contact_collection(cards_server, "AutoOffIslandB")
    a_ids = [make_contact(cards_server, coll_a, f"A{i}") for i in range(3)]
    b_ids = [make_contact(cards_server, coll_b, f"B{i}") for i in range(3)]
    move_contacts_to(cards_server, [
        pos(a_ids[0], 0, 0), pos(a_ids[1], 0, 1), pos(a_ids[2], 20, 0),
        pos(b_ids[0], 60, 3), pos(b_ids[1], 60, 4), pos(b_ids[2], 80, 3),
    ])

    page = cards_page
    page.goto(f"{cards_server.url}/contacts", wait_until="load")
    page.wait_for_selector("[data-contact-id]", timeout=15_000)
    page.set_viewport_size({"width": 1400, "height": 1000})
    page.wait_for_timeout(300)

    # One real drag first, matching actual usage -- not just seeded data.
    a0_rect = card_rects(page, "[data-contact-id]")[a_ids[0]]
    to_x = a0_rect["x"] + a0_rect["w"] / 2 + 20
    to_y = a0_rect["y"] + a0_rect["h"] + 60
    drag_card(page, f"[data-contact-id='{a_ids[0]}']", to_x, to_y, "move_contacts")
    page.wait_for_timeout(1000)

    baseline = card_positions(page, "[data-contact-id]")
    assert_no_overlaps(page, "[data-contact-id]")
    stored_before = {c["id"]: (c["grid_row"], c["grid_col"]) for c in list_contacts(cards_server)}

    page.click("button:has-text('Auto-layout')")
    page.wait_for_timeout(500)
    during = card_positions(page, "[data-contact-id]")
    assert during != baseline, "calibration: Auto-layout preview must actually repack"

    page.click("button:has-text('Auto-layout')")
    page.wait_for_timeout(500)
    after = card_positions(page, "[data-contact-id]")

    for ids in (a_ids, b_ids):
        after_cols = {after[i][1] for i in ids}
        assert max(after_cols) - min(after_cols) <= len(ids) - 1, (
            f"island {ids} must stay column-compact after toggling Auto-layout off: {after}"
        )
    a_after_cols = {after[i][1] for i in a_ids}
    b_after_cols = {after[i][1] for i in b_ids}
    a_after_rows = {after[i][0] for i in a_ids}
    b_after_rows = {after[i][0] for i in b_ids}
    assert not (a_after_rows & b_after_rows and a_after_cols & b_after_cols), (
        f"the two islands must never end up mixed together after toggling Auto-layout off: {after}"
    )
    assert_no_overlaps(page, "[data-contact-id]")

    # Nothing must ever be persisted while only previewing -- ground truth
    # is the DB state from just before opening the preview (`stored_before`),
    # not the rendered `baseline` DOM snapshot: Manual mode always renders
    # a *cluster-packed* view of the stored positions (backlog #102), which
    # can legitimately differ from an untouched member's own literal stored
    # (row, col) even with no preview involved at all.
    stored_after = {c["id"]: (c["grid_row"], c["grid_col"]) for c in list_contacts(cards_server)}
    for cid in a_ids + b_ids:
        assert stored_after[cid] == stored_before[cid], (
            f"Auto-layout preview must never persist: contact {cid} "
            f"before={stored_before[cid]} after={stored_after[cid]}"
        )


# ── 20. A resize from the maximum to the minimum column count and back must
# preserve gaggles (backlog #106 follow-up, reported live 2026-09-27) ─────────

def test_resize_from_max_to_min_columns_and_back_preserves_gaggles(cards_server, cards_page):
    coll_a = make_contact_collection(cards_server, "ExtremeIslandA")
    coll_b = make_contact_collection(cards_server, "ExtremeIslandB")
    a_ids = [make_contact(cards_server, coll_a, f"EA{i}") for i in range(3)]
    b_ids = [make_contact(cards_server, coll_b, f"EB{i}") for i in range(3)]
    move_contacts_to(cards_server, [
        pos(a_ids[0], 0, 0), pos(a_ids[1], 0, 1), pos(a_ids[2], 20, 0),
        pos(b_ids[0], 60, 3), pos(b_ids[1], 60, 4), pos(b_ids[2], 80, 3),
    ])

    page = cards_page
    page.goto(f"{cards_server.url}/contacts", wait_until="load")
    page.wait_for_selector("[data-contact-id]", timeout=15_000)
    page.set_viewport_size({"width": 3000, "height": 1000})
    page.wait_for_timeout(300)
    # `.contacts-main` itself caps out around 2000px wide regardless of
    # viewport (CSS max-width) -- 6, not `grid::CARD_GRID_COLS` (8), is
    # the actual maximum column count reachable via resizing alone.
    assert rendered_cols(page) == 6, "calibration: this test needs the max reachable column count at 3000px"

    # One real drag first, matching actual usage -- not just seeded data.
    a0_rect = card_rects(page, "[data-contact-id]")[a_ids[0]]
    to_x = a0_rect["x"] + a0_rect["w"] / 2 + 20
    to_y = a0_rect["y"] + a0_rect["h"] + 60
    drag_card(page, f"[data-contact-id='{a_ids[0]}']", to_x, to_y, "move_contacts")
    page.wait_for_timeout(1000)
    assert_no_overlaps(page, "[data-contact-id]")

    # A drag's own on-screen result is patched in directly, exact render-
    # space precision (so a drop lands exactly where its shadow showed,
    # backlog #106's original fix) -- it is *not* itself
    # `pack_clusters_for_view`'s own canonical output, so the very first
    # resize settles it into that canonical form (same contract as "Keep
    # this layout": see its own doc comment). One settling resize round
    # trip first, then `baseline` is captured *after* that -- this is what
    # a second, identical round trip actually has to reproduce exactly.
    page.set_viewport_size({"width": 500, "height": 1000})
    page.wait_for_timeout(300)
    assert rendered_cols(page) == 1, "calibration: this test needs the min column count at 500px"
    page.set_viewport_size({"width": 3000, "height": 1000})
    page.wait_for_timeout(300)
    baseline = card_positions(page, "[data-contact-id]")

    page.set_viewport_size({"width": 500, "height": 1000})
    page.wait_for_timeout(300)
    assert_no_overlaps(page, "[data-contact-id]")

    page.set_viewport_size({"width": 3000, "height": 1000})
    page.wait_for_timeout(300)

    # Exact per-id position equality (backlog #106 fourth follow-up): see
    # the identical comment in
    # test_manual_layout_survives_a_resize_round_trip_after_real_drags.
    after = card_positions(page, "[data-contact-id]")
    assert after == baseline, (
        f"a max -> min -> max resize round trip must reproduce the exact same layout: "
        f"baseline={baseline} after={after}"
    )
    assert_no_overlaps(page, "[data-contact-id]")


# ── 21. Same as #19, but with four gaggles (denser, forcing multiple
# shelves) -- checking whether more gaggles expose merging #19 didn't ────────

def test_auto_layout_off_restores_four_manual_gaggles(cards_server, cards_page):
    colls = [make_note_collection(cards_server, f"Gaggle{i}") for i in range(4)]
    id_groups = [[make_note(cards_server, colls[i], f"G{i}N{j}") for j in range(3)] for i in range(4)]
    positions = []
    for i, ids in enumerate(id_groups):
        row_base = i * 100
        positions += [pos(ids[0], row_base, 0), pos(ids[1], row_base, 1), pos(ids[2], row_base + 20, 0)]
    move_notes_to(cards_server, positions)

    page = cards_page
    page.goto(f"{cards_server.url}/notes", wait_until="load")
    page.wait_for_selector("[data-note-id]", timeout=15_000)
    page.set_viewport_size({"width": 1400, "height": 1000})
    page.wait_for_timeout(300)

    # One real drag, matching actual usage -- not just seeded data.
    d0_rect = card_rects(page, "[data-note-id]")[id_groups[0][0]]
    to_x = d0_rect["x"] + d0_rect["w"] / 2 + 20
    to_y = d0_rect["y"] + d0_rect["h"] + 60
    drag_card(page, f"[data-note-id='{id_groups[0][0]}']", to_x, to_y, "move_notes")
    page.wait_for_timeout(1000)

    baseline = card_positions(page, "[data-note-id]")
    assert_no_overlaps(page, "[data-note-id]")

    page.click("button:has-text('Auto-layout')")
    page.wait_for_timeout(500)
    during = card_positions(page, "[data-note-id]")
    assert during != baseline, "calibration: Auto-layout preview must actually repack"

    page.click("button:has-text('Auto-layout')")
    page.wait_for_timeout(500)
    after = card_positions(page, "[data-note-id]")

    for ids in id_groups:
        after_cols = {after[i][1] for i in ids}
        assert max(after_cols) - min(after_cols) <= len(ids) - 1, (
            f"gaggle {ids} must stay column-compact after toggling Auto-layout off: {after}"
        )
    for i in range(len(id_groups)):
        for j in range(i + 1, len(id_groups)):
            rows_i = {after[k][0] for k in id_groups[i]}
            cols_i = {after[k][1] for k in id_groups[i]}
            rows_j = {after[k][0] for k in id_groups[j]}
            cols_j = {after[k][1] for k in id_groups[j]}
            assert not (rows_i & rows_j and cols_i & cols_j), (
                f"gaggles {id_groups[i]} and {id_groups[j]} must never end up mixed together "
                f"after toggling Auto-layout off: {after}"
            )
    assert_no_overlaps(page, "[data-note-id]")


# ── 22. Same as #20, but with four gaggles across several resize steps ───────

def test_resize_through_several_widths_preserves_four_manual_gaggles(cards_server, cards_page):
    colls = [make_note_collection(cards_server, f"RGaggle{i}") for i in range(4)]
    id_groups = [[make_note(cards_server, colls[i], f"R{i}N{j}") for j in range(3)] for i in range(4)]
    positions = []
    for i, ids in enumerate(id_groups):
        row_base = i * 100
        positions += [pos(ids[0], row_base, 0), pos(ids[1], row_base, 1), pos(ids[2], row_base + 20, 0)]
    move_notes_to(cards_server, positions)

    page = cards_page
    page.goto(f"{cards_server.url}/notes", wait_until="load")
    page.wait_for_selector("[data-note-id]", timeout=15_000)
    page.set_viewport_size({"width": 1400, "height": 1000})
    page.wait_for_timeout(300)

    d0_rect = card_rects(page, "[data-note-id]")[id_groups[0][0]]
    to_x = d0_rect["x"] + d0_rect["w"] / 2 + 20
    to_y = d0_rect["y"] + d0_rect["h"] + 60
    drag_card(page, f"[data-note-id='{id_groups[0][0]}']", to_x, to_y, "move_notes")
    page.wait_for_timeout(1000)
    assert_no_overlaps(page, "[data-note-id]")

    # A drag's own on-screen result is patched in directly, exact render-
    # space precision -- it is *not* itself `pack_clusters_for_view`'s own
    # canonical output, so the very first resize settles it into that
    # canonical form (same contract as "Keep this layout"). One settling
    # resize round trip first, then `baseline` is captured *after* that --
    # see the identical comment in
    # test_manual_layout_survives_a_resize_round_trip_after_real_drags.
    page.set_viewport_size({"width": 900, "height": 1000})
    page.wait_for_timeout(300)
    page.set_viewport_size({"width": 1400, "height": 1000})
    page.wait_for_timeout(300)
    baseline = card_positions(page, "[data-note-id]")

    for width in (900, 600, 1150, 500, 1400):
        page.set_viewport_size({"width": width, "height": 1000})
        page.wait_for_timeout(300)
        assert_no_overlaps(page, "[data-note-id]")

    # Exact per-id position equality (backlog #106 fourth follow-up) --
    # the loop above ends back at 1400, the same width `baseline` was
    # captured at.
    after = card_positions(page, "[data-note-id]")
    assert after == baseline, (
        f"resizing through several widths and back must reproduce the exact same layout: "
        f"baseline={baseline} after={after}"
    )


# ── 23. Gaggles built via several real manual drags (not seeded positions)
# must survive an Auto-layout on/off round trip ───────────────────────────────

def test_auto_layout_off_restores_gaggles_built_via_several_real_drags(cards_server, cards_page):
    colls = [make_note_collection(cards_server, f"DragGaggle{i}") for i in range(3)]
    id_groups = [[make_note(cards_server, colls[i], f"D{i}N{j}") for j in range(3)] for i in range(3)]
    # Start scattered -- default sequential placement, deliberately NOT
    # pre-clustered -- so every gaggle is actually built by dragging, the
    # same way a real user assembles one. Columns start at 1, not 0 (backlog
    # #106 sixth follow-up, found live): the notes grid container has its
    # own hard max-width, capping `cols_now` at 7 real columns no matter how
    # wide the viewport below is set, and `group_of(real_col, cols_now)`
    # aliases real columns 0 and 1 into the very same view column at
    # `cols_now == 7` (every other adjacent pair maps distinctly) --
    # `column_offsets` then concatenates the two real columns' entire row
    # ranges one after another rather than interleaving them (an
    # inherent, correct consequence of `compact_cell`/`expand_cell` being a
    # true bijection: preserving each real column's own internal spacing
    # exactly is what a masonry-style interleave would have to break), which
    # pushed the second real column's members thousands of pixels down the
    # page -- one ends up rendered far below any reasonable viewport height,
    # so its own `page.mouse` drag (which doesn't auto-scroll) never reaches
    # it and the resulting `move_notes` call never fires. Real columns 1-6
    # all map to distinct view columns at `cols_now == 7`.
    flat_ids = [nid for group in id_groups for nid in group]
    move_notes_to(cards_server, [pos(nid, i * 20, 1 + i % 6) for i, nid in enumerate(flat_ids)])

    page = cards_page
    page.goto(f"{cards_server.url}/notes", wait_until="load")
    page.wait_for_selector("[data-note-id]", timeout=15_000)
    # Wide enough to reach the notes container's own max-width (so
    # `cols_now` hits its ceiling of 7, matching the column choice above),
    # and tall enough that every drop target below stays within the
    # viewport -- a target past the viewport's edge is unreachable by raw
    # (non-element-targeted) `page.mouse` coordinates, which don't
    # auto-scroll the page the way an element-targeted Playwright action
    # would.
    page.set_viewport_size({"width": 2400, "height": 2000})
    page.wait_for_timeout(300)

    # Drag every non-anchor member of each group onto (near) its own
    # group's anchor (first member), one real drag at a time -- each
    # member offset a little further below the anchor than the last, so
    # sequential drops within one group never target the exact same cell
    # a previous drop already landed on (an unrelated ambiguity, not what
    # this test is about).
    for group in id_groups:
        anchor = group[0]
        for offset, member in enumerate(group[1:], start=1):
            anchor_rect = card_rects(page, "[data-note-id]")[anchor]
            to_x = anchor_rect["x"] + anchor_rect["w"] / 2
            to_y = anchor_rect["y"] + anchor_rect["h"] * offset + 4
            drag_card(page, f"[data-note-id='{member}']", to_x, to_y, "move_notes")
            page.wait_for_timeout(400)
    page.wait_for_timeout(1000)

    baseline = card_positions(page, "[data-note-id]")
    assert_no_overlaps(page, "[data-note-id]")
    for ids in id_groups:
        baseline_cols = {baseline[i][1] for i in ids}
        assert max(baseline_cols) - min(baseline_cols) <= len(ids) - 1, (
            f"calibration: gaggle {ids} must already be column-compact right after being "
            f"dragged together: {baseline}"
        )

    page.click("button:has-text('Auto-layout')")
    page.wait_for_timeout(500)
    during = card_positions(page, "[data-note-id]")
    assert during != baseline, "calibration: Auto-layout preview must actually repack"

    page.click("button:has-text('Auto-layout')")
    page.wait_for_timeout(500)
    after = card_positions(page, "[data-note-id]")

    for ids in id_groups:
        after_cols = {after[i][1] for i in ids}
        assert max(after_cols) - min(after_cols) <= len(ids) - 1, (
            f"gaggle {ids} must stay column-compact after toggling Auto-layout off: {after}"
        )
    for i in range(len(id_groups)):
        for j in range(i + 1, len(id_groups)):
            rows_i = {after[k][0] for k in id_groups[i]}
            cols_i = {after[k][1] for k in id_groups[i]}
            rows_j = {after[k][0] for k in id_groups[j]}
            cols_j = {after[k][1] for k in id_groups[j]}
            assert not (rows_i & rows_j and cols_i & cols_j), (
                f"gaggles {id_groups[i]} and {id_groups[j]} must never end up mixed together "
                f"after toggling Auto-layout off: {after}"
            )
    assert_no_overlaps(page, "[data-note-id]")


# ── 24. A height self-correction (mismatched placeholder span) must never
# reorder gaggles relative to each other on a later resize (backlog #106
# follow-up, reported live 2026-09-27 -- "resizing down and back loses the
# gaggles' horizontal/shelf position, everything ends up centered") ──────────

def test_height_correction_never_reorders_gaggles_on_a_later_resize(cards_server, cards_page):
    coll_a = make_note_collection(cards_server, "CorrectionIslandA")
    coll_b = make_note_collection(cards_server, "CorrectionIslandB")
    # Deliberately long bodies so the real rendered height is much taller
    # than the placeholder `span=1` seeded below -- guaranteed to trigger
    # the height self-correction pass on first load.
    long_text = "Line one of several.\n" * 8
    a_ids = [make_note(cards_server, coll_a, f"A{i}: {long_text}") for i in range(2)]
    b_ids = [make_note(cards_server, coll_b, f"B{i}") for i in range(2)]
    # A ranks first (row 0) only because of its seeded row -- once its own
    # height correction pushes real content down, this is exactly the
    # scenario that could otherwise flip A and B's relative shelf order.
    move_notes_to(cards_server, [
        pos(a_ids[0], 0, 0, span=1), pos(a_ids[1], 0, 1, span=1),
        pos(b_ids[0], 40, 0), pos(b_ids[1], 40, 1),
    ])

    page = cards_page
    page.goto(f"{cards_server.url}/notes", wait_until="load")
    page.wait_for_selector("[data-note-id]", timeout=15_000)
    page.set_viewport_size({"width": 1400, "height": 1000})
    # Let the RAF-scheduled height self-correction pass fully settle.
    page.wait_for_timeout(1500)
    assert_no_overlaps(page, "[data-note-id]")

    def shelf_order(positions):
        a_row = min(positions[i][0] for i in a_ids)
        b_row = min(positions[i][0] for i in b_ids)
        a_col = min(positions[i][1] for i in a_ids)
        b_col = min(positions[i][1] for i in b_ids)
        return (a_row, a_col) < (b_row, b_col) if a_row != b_row else a_col < b_col

    baseline = card_positions(page, "[data-note-id]")
    baseline_a_first = shelf_order(baseline)

    for width in (900, 600, 1400):
        page.set_viewport_size({"width": width, "height": 1000})
        page.wait_for_timeout(300)
        assert_no_overlaps(page, "[data-note-id]")

    after = card_positions(page, "[data-note-id]")
    for ids in (a_ids, b_ids):
        after_cols = {after[i][1] for i in ids}
        assert max(after_cols) - min(after_cols) <= len(ids) - 1, (
            f"island {ids} must stay column-compact: {after}"
        )
    assert shelf_order(after) == baseline_a_first, (
        f"a height correction must never change which gaggle ranks first, on any later "
        f"resize: baseline={baseline} after={after}"
    )


# ── 24. A large gaggle must not claim an entire shelf's width and crowd a
# smaller sibling gaggle onto its own separate shelf (backlog #106 fourth
# follow-up) ───────────────────────────────────────────────────────────────

def test_large_gaggle_does_not_crowd_a_smaller_sibling_off_its_shelf(cards_server, cards_page):
    big_coll = make_note_collection(cards_server, "BigGaggle")
    small_coll = make_note_collection(cards_server, "SmallGaggle")
    big_ids = [make_note(cards_server, big_coll, f"Big{i}") for i in range(8)]
    small_ids = [make_note(cards_server, small_coll, f"Small{i}") for i in range(3)]
    # Seeded side by side, same row band, deliberately compact (2 columns
    # for the big gaggle, not spread across all 8) -- exactly what a real
    # user's manual drag arrangement looks like: a `pack_shortest_column`
    # masonry balance across `len(members)` columns, rather than the
    # column span actually used, would stretch the 8-member gaggle across
    # up to 8 columns, leaving no room for the small gaggle beside it.
    move_notes_to(cards_server, [
        pos(big_ids[0], 0, 0), pos(big_ids[1], 0, 1),
        pos(big_ids[2], 16, 0), pos(big_ids[3], 16, 1),
        pos(big_ids[4], 32, 0), pos(big_ids[5], 32, 1),
        pos(big_ids[6], 48, 0), pos(big_ids[7], 48, 1),
        pos(small_ids[0], 0, 3), pos(small_ids[1], 0, 4), pos(small_ids[2], 16, 3),
    ])

    page = cards_page
    page.goto(f"{cards_server.url}/notes", wait_until="load")
    page.wait_for_selector("[data-note-id]", timeout=15_000)
    # Wide enough for both gaggles' own natural width (2 columns each,
    # plus a 1-column buffer) to actually fit on one shelf together.
    page.set_viewport_size({"width": 2200, "height": 1000})
    # Let the RAF-scheduled height self-correction pass fully settle
    # before taking any snapshot -- these short-text seed notes commonly
    # measure shorter than their placeholder span.
    page.wait_for_timeout(1500)
    assert_no_overlaps(page, "[data-note-id]")

    def side_by_side(positions):
        big_rows = {positions[i][0] for i in big_ids}
        small_rows = {positions[i][0] for i in small_ids}
        big_cols = {positions[i][1] for i in big_ids}
        small_cols = {positions[i][1] for i in small_ids}
        # They share at least one row band (same shelf) and occupy
        # disjoint column ranges (side by side, not stacked on top of
        # each other with both starting at column 0).
        return bool(big_rows & small_rows) and not (big_cols & small_cols)

    baseline = card_positions(page, "[data-note-id]")
    assert side_by_side(baseline), (
        f"calibration: the two gaggles must start out side by side: {baseline}"
    )

    page.set_viewport_size({"width": 900, "height": 1000})
    page.wait_for_timeout(300)
    page.set_viewport_size({"width": 2200, "height": 1000})
    page.wait_for_timeout(300)

    after = card_positions(page, "[data-note-id]")
    assert side_by_side(after), (
        f"the small gaggle must still sit beside the large one after a resize round "
        f"trip, not be crowded onto its own separate shelf: baseline={baseline} after={after}"
    )







def test_keep_this_layout_matches_the_preview_with_pinned_notes_present(cards_server, cards_page):
    # Reported live 2026-09-28: "Keep this layout doesn't keep the same
    # layout when you have some cards pinned." Root-caused to two separate
    # bugs specific to the pinned/unpinned split:
    # 1. `on_keep_layout` batched pinned and unpinned notes into a single
    #    `move_notes` call, which the server rejects outright ("Cannot
    #    move notes across the pinned boundary in one batch") -- silently
    #    discarding the *entire* commit (both zones) whenever at least one
    #    note of each kind was involved.
    # 2. Even after splitting that into two calls, the pinned row's own
    #    CSS grid reserved height for a stale, pre-height-correction span
    #    during the Auto-layout preview (`pack_for_view`'s entries came
    #    straight from `notes_res`'s raw, deliberately-never-refetched
    #    copy), while the post-commit manual render used the corrected
    #    span -- visibly shifting the notes grid below the pinned row by
    #    the difference.
    coll = make_note_collection(cards_server, "PinGroup")
    pinned_ids = [make_note(cards_server, coll, f"P{i}", pinned=True) for i in range(2)]
    other_ids = [make_note(cards_server, coll, f"O{i}", pinned=False) for i in range(3)]
    all_ids = pinned_ids + other_ids
    move_notes_to(cards_server, [pos(nid, i * 10, i % 6) for i, nid in enumerate(pinned_ids)])
    move_notes_to(cards_server, [pos(nid, i * 10, i % 6) for i, nid in enumerate(other_ids)])

    page = cards_page
    page.goto(f"{cards_server.url}/notes", wait_until="load")
    page.wait_for_selector("[data-note-id]", timeout=15_000)
    page.set_viewport_size({"width": 1400, "height": 1000})
    page.wait_for_timeout(300)

    page.click("button:has-text('Auto-layout')")
    page.wait_for_timeout(800)
    previewed = card_rects(page, "[data-note-id]")
    assert set(previewed) == set(all_ids), "every note must be visible during the Auto-layout preview"

    with page.expect_response(lambda r: "move_notes" in r.url, timeout=5_000):
        page.click("button:has-text('Keep this layout')")
    page.wait_for_timeout(300)
    kept = card_rects(page, "[data-note-id]")

    for nid in all_ids:
        before, after = previewed[nid], kept.get(nid)
        assert after is not None and (after["x"], after["y"]) == (before["x"], before["y"]), (
            f"'Keep this layout' must commit exactly what was just previewed for "
            f"note {nid} (pinned={nid in pinned_ids}): previewed={before} kept={after}"
        )
    assert_no_overlaps(page, "[data-note-id]")


def test_zzz_debug_real_scale_pinning(cards_server, cards_page):
    coll = make_note_collection(cards_server, "RealScale")
    texts = [
        ("TODO: renew passport", "Current one expires March 2027 but some countries want 6mo validity — check requirements before booking the June trip. Bring old passport + photo.", True),
        ("dwd", "eef", False),
        ("efe", "fefefef", False),
        ("TODO: car service", "Due at 45,000km or next month, whichever first.\nAsk about the brake pad wear while it's in.", False),
        ("Movie watchlist", "- Arrival (rewatch)\n- Perfect Days\n- The Zone of Interest\n- Poor Things\n- Anatomy of a Fall", False),
        ("Grocery list", "Grocery list:\n- oat milk\n- eggs\n- sourdough flour\n- parmesan\n- dark chocolate 85%\n- coffee beans", False),
        ("TODO: taxes prep", "Gather:\n- mortgage interest statement\n- freelance invoices\n- charity receipts\nAppointment with accountant tentatively booked for late Feb.", False),
        ("Gift ideas", "Bob: that ceramics class he mentioned\nDad: the fountain pen ink set\nMax: board game — Wingspan expansion?", False),
        ("efdef", "efef", False),
        ("efe2", "fefef", False),
        ("Book list 2026", "- Project Hail Mary (re-read)\n- Piranesi\n- The Overstory\n- Klara and the Sun\n- A Canticle for Leibowitz", False),
        ("Shared: plant watering rota", "Mon/Thu: living room ferns\nWed: balcony pots\nSat: kitchen herbs\nWhoever's around that day, no need to swap explicitly.", False),
        ("TODO: backup NAS config", "Export the docker-compose files and the Samba share config before the firmware update.\nLast backup: unknown — check before assuming it's covered.", False),
        ("Running log notes", "Easy pace should feel conversational, ~5:45/km.\nLong run every Sunday, +10% distance week over week, cutback every 4th week.", False),
        ("Shared: holiday planning", "Thinking early October for the cabin trip. Need to confirm with the others.\nBudget ballpark: 800/person for 4 nights.", False),
        ("Wifi guest network", "SSID: Example-Guest\nPassword: example-wifi-password-42\nExpires: never, but rotate yearly.", False),
        ("Sourdough starter feeding schedule", "Feed 1:1:1 (starter:flour:water) every 12h at room temp.\nIf it's sluggish, move to a warmer spot (~24C) and feed a bit more often.\nDiscard before feeding unless building up for a bake.", False),
        ("efef", "efef", False),
        ("Home maintenance log", "2026-06: replaced furnace filter\n2026-07: gutters cleaned\n2026-08: smoke detector batteries\nNext: check water softener salt level.", False),
        ("Shared: house rules draft", "- Quiet hours after 22:00\n- Dishes done same day\n- Guests: heads up 24h in advance\nFeel free to edit — @Bob can you add the recycling schedule?", False),
    ]
    ids = [make_note(cards_server, coll, t, pinned=p) for _, t, p in [(title, f"{title}\n{body}" if title else body, pinned) for title, body, pinned in texts]]
    pinned_positions = [(i, nid) for i, (nid, (_, _, p)) in enumerate(zip(ids, texts)) if p]
    other_positions = [(i, nid) for i, (nid, (_, _, p)) in enumerate(zip(ids, texts)) if not p]
    if pinned_positions:
        move_notes_to(cards_server, [pos(nid, i * 5, i % 8) for i, nid in pinned_positions])
    if other_positions:
        move_notes_to(cards_server, [pos(nid, i * 5, i % 8) for i, nid in other_positions])

    page = cards_page
    page.on("console", lambda msg: print("CONSOLE:", msg.text))
    page.goto(f"{cards_server.url}/notes", wait_until="load")
    page.wait_for_selector("[data-note-id]", timeout=15_000)
    page.set_viewport_size({"width": 2500, "height": 1200})
    page.wait_for_timeout(500)

    page.click("button:has-text('Auto-layout')")
    page.wait_for_timeout(1000)
    previewed = card_rects(page, "[data-note-id]")
    print("PREVIEWED count:", len(previewed), "of", len(ids))
    missing_preview = set(ids) - set(previewed)
    print("MISSING IN PREVIEW:", missing_preview)

    with page.expect_response(lambda r: "move_notes" in r.url, timeout=5_000):
        page.click("button:has-text('Keep this layout')")
    page.wait_for_timeout(500)
    kept = card_rects(page, "[data-note-id]")
    print("KEPT count:", len(kept), "of", len(ids))
    missing_kept = set(ids) - set(kept)
    print("MISSING IN KEPT:", missing_kept)

    mismatches = []
    for nid in ids:
        if nid not in previewed or nid not in kept:
            continue
        b, a = previewed[nid], kept[nid]
        if (round(b["x"],1), round(b["y"],1)) != (round(a["x"],1), round(a["y"],1)):
            mismatches.append((nid, (b["x"],b["y"]), (a["x"],a["y"])))
    print("MISMATCHES:", mismatches)

    stored = {n["id"]: (n["grid_row"], n["grid_col"], n["grid_row_span"], n["pinned"]) for n in list_notes(cards_server)}
    print("STORED:", stored)
