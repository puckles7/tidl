import os

import pytest
import caldav
from playwright.sync_api import sync_playwright

from fixtures import TidlServer, tidl_binary  # noqa: F401 — tidl_binary re-exported as fixture

# A stray LD_LIBRARY_PATH from the host session (pipewire/alsa under nix-ld)
# force-loads a newer-glibc libasound.so.2 into Playwright's bundled Chromium,
# which links glibc 2.40, so it crashes at launch with `GLIBC_ABI_GNU2_TLS not
# found`. The Nix dev shell already unsets it; this covers pytest runs started
# from outside the shell (e.g. an IDE test runner).
os.environ.pop("LD_LIBRARY_PATH", None)


def pytest_addoption(parser):
    parser.addoption("--server-url", default="http://localhost:3060")
    parser.addoption("--username",   default="admin")
    parser.addoption("--password",   default="admin")
    # Web UI login uses an email address rather than the CalDAV username.
    # Defaults to None; test_hydration.py falls back to --username when unset.
    parser.addoption("--email",      default=None)


@pytest.fixture(scope="session")
def tidl_server(tidl_binary):
    """Start a tidl-server on port 3060 for the CalDAV and hydration test suites."""
    srv = TidlServer(tidl_binary, port=3060, username="admin", password="admin")
    srv.start()
    yield srv
    srv.stop()


@pytest.fixture(scope="session")
def server_url(request, tidl_server):
    return request.config.getoption("--server-url").rstrip("/")


@pytest.fixture(scope="session")
def credentials(request):
    return (
        request.config.getoption("--username"),
        request.config.getoption("--password"),
    )

@pytest.fixture(scope="session")
def principal_url(server_url, credentials):
    return f"{server_url}/caldav/{credentials[0]}/"

@pytest.fixture(scope="session")
def dav_client(principal_url, credentials):
    return caldav.DAVClient(
        url=principal_url,
        username=credentials[0],
        password=credentials[1],
    )

@pytest.fixture(scope="session")
def principal(dav_client):
    return dav_client.principal()


@pytest.fixture(scope="session")
def browser():
    """Single session-scoped headless Chromium browser shared across all test suites.

    Only one sync_playwright() context can be active per process; centralising
    it here prevents the hydration and sync test suites from each creating their
    own conflicting event loops.
    """
    with sync_playwright() as pw:
        b = pw.chromium.launch(headless=True)
        yield b
        b.close()
