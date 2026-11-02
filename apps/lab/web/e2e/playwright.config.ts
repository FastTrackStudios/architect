import { defineConfig, devices } from "@playwright/test";

/**
 * These run against `../dist`, which `just lab-web-build` makes (the host
 * built for wasm32 and bound with wasm-bindgen, its page and the page's
 * own Tailwind sheet), served by `../serve.mjs`. `just lab-web-e2e`
 * builds it first; nothing here builds Rust.
 *
 * The page is a Dioxus app, so every test waits for the surface to be
 * drawn before it touches it: the agent renders it once the app is up,
 * and `window.a2ui` (the page's door to the Action List, as that agent)
 * only answers after that.
 */
const PORT = Number(process.env.LAB_WEB_PORT ?? 8282);
const BASE_URL = `http://127.0.0.1:${PORT}`;

/**
 * A browser to use instead of the one Playwright downloads: on NixOS a
 * downloaded binary cannot find its libraries (as in
 * apps/auth-server/e2e).
 *
 *   PLAYWRIGHT_CHROMIUM_PATH=$(which chromium) just lab-web-e2e
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
    trace: "on-first-retry",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"], launchOptions } }],
  webServer: {
    command: "node ../serve.mjs",
    url: BASE_URL,
    env: { LAB_WEB_PORT: String(PORT) },
    reuseExistingServer: !process.env.CI,
    timeout: 30_000,
  },
});
