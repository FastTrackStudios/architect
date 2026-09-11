import { test as base, expect, type Page } from "@playwright/test";

/**
 * The cast `AUTH_DEV_SEED=1` creates. Hard-coded because the seed is
 * deterministic — the same people and organizations on every machine
 * and every run — which is the whole reason these assertions can name
 * `ada` rather than "whichever user came back first".
 *
 * Kept in step with `apps/auth-server/src/dev.rs`; its own unit tests
 * check the table there is internally consistent.
 */
export const PASSWORD = "development-password";

export const PEOPLE = {
  /** Server administrator. Owner of Acme Records. */
  ada: "ada@local.test",
  /** Admin of Acme Records, owner of Indie Collective. */
  grace: "grace@local.test",
  /** Plain member of Indie Collective. */
  alan: "alan@local.test",
} as const;

export const ORGS = {
  acme: "Acme Records",
  indie: "Indie Collective",
} as const;

/**
 * The working surface, excluding the settings rail.
 *
 * The rail repeats the organizations and "Sign out", so a page-wide
 * `getByRole` matches twice. Scoping to `main` is also closer to what
 * these tests mean: "the thing the page is about".
 */
export function sheet(page: Page) {
  return page.locator("main");
}

/** Sign in through the real form, the way a person does. */
export async function signIn(page: Page, email: string): Promise<void> {
  await page.goto("/login");
  await page.getByLabel(/email/i).fill(email);
  await page.getByLabel(/password/i).fill(PASSWORD);
  // Exact: the login page also offers "Sign in with a passkey".
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  // The redirect off /login is the signal; asserting on it here means a
  // broken sign-in fails in the fixture rather than confusingly later.
  await expect(page).not.toHaveURL(/\/login/);
}

/**
 * A brand-new account, for a test that changes something about the
 * person themselves.
 *
 * The suite runs in parallel against ONE server, so a test that revokes
 * a session or changes a password must not do it to somebody another
 * test is signed in as. Anything touching credentials gets its own
 * account rather than borrowing one from the seed.
 */
export async function signUpFresh(page: Page): Promise<string> {
  const email = `e2e-${crypto.randomUUID()}@local.test`;
  await page.goto("/sign-up");
  await page.getByLabel("Email").fill(email);
  await page.getByLabel("Password").fill(PASSWORD);
  await page.getByRole("button", { name: /create account/i }).click();
  await expect(page).not.toHaveURL(/\/sign-up/);
  return email;
}

/** A signed-out browser, for the invitation and join pages. */
export const test = base.extend<{ signedIn: Page }>({
  signedIn: async ({ page }, use) => {
    await signIn(page, PEOPLE.ada);
    await use(page);
  },
});

export { expect };
