import { defineConfig, devices } from "@playwright/test";

/**
 * These run against a server this config starts: `AUTH_DEV_SEED=1` with
 * an in-memory database, so every run begins from the same fixed cast
 * and nothing has to be torn down afterwards.
 *
 * The pages are server-rendered with no JavaScript of their own, which
 * is why there is no `waitForHydration` anywhere below: a form post
 * either redirected or it did not, and the next page is complete when
 * it arrives. That also means these tests are fast enough to run on
 * every push rather than nightly.
 */
const PORT = Number(process.env.AUTH_E2E_PORT ?? 8181);
// `localhost`, not `127.0.0.1`: a WebAuthn relying-party id has to be a
// registrable domain suffix of the origin's host, and an IP address has
// no such suffix — passkeys simply cannot work on one.
const BASE_URL = `http://localhost:${PORT}`;

/**
 * A browser to use instead of the one Playwright downloads.
 *
 * Needed on NixOS, where a downloaded binary cannot find `libglib` and
 * friends — there is no global lib directory for it to find them in.
 * Point this at a system browser instead:
 *
 *   nix-shell -p chromium --run \
 *     'PLAYWRIGHT_CHROMIUM_PATH=$(which chromium) npm test'
 *
 * Unset everywhere else, where the downloaded browser is the right one
 * to use because it is the version this Playwright was tested against.
 */
const CHROMIUM = process.env.PLAYWRIGHT_CHROMIUM_PATH;
const launchOptions = CHROMIUM ? { executablePath: CHROMIUM } : {};

export default defineConfig({
  testDir: "./tests",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  reporter: process.env.CI ? "github" : "list",
  use: {
    baseURL: BASE_URL,
    // On the first retry, so a flake in CI arrives with a trace rather
    // than a line number.
    trace: "on-first-retry",
  },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"], launchOptions } },
    // The pages must work on a phone: an invite link is followed on
    // whatever device the message arrived on.
    { name: "mobile", use: { ...devices["Pixel 7"], launchOptions } },
  ],
  webServer: {
    // `--release` would be faster to run and much slower to build; these
    // are IO-bound against sqlite either way.
    command: "cargo run -p auth-server",
    url: `${BASE_URL}/healthz`,
    reuseExistingServer: !process.env.CI,
    timeout: 300_000,
    cwd: "../../..",
    env: {
      AUTH_DEV_SEED: "1",
      AUTH_DATABASE_URL: "sqlite::memory:",
      AUTH_SECRET: "a-secret-at-least-32-bytes-long!!",
      AUTH_BASE_URL: BASE_URL,
      AUTH_BIND_ADDR: `127.0.0.1:${PORT}`,
      AUTH_PASSKEY_RP_ID: "localhost",
      RUST_LOG: "info,auth_server=debug",
    },
  },
});
