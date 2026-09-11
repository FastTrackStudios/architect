import type { Page } from "@playwright/test";

import { expect, sheet, signUpFresh, test } from "./fixtures";

/**
 * Social sign-in, through the mock provider the config starts.
 *
 * The mock is a real server on its own origin, so these tests cross a
 * redirect out of the app and back — which is the part that cannot be
 * covered without one. What they are checking is the seam: the button
 * leaves, the provider answers, and the person comes back as somebody.
 */

/** The account picker the mock renders. Pick a name and come back. */
async function pickOnProvider(page: Page, name: RegExp) {
  await expect(page.getByRole("heading", { name: /continue with/i })).toBeVisible();
  await page.getByRole("button", { name }).click();
}

/**
 * Come back as somebody nobody else is.
 *
 * The two browser projects run the same file at the same time against
 * one server, so both linking the mock's `ada.lovelace` would leave the
 * slower one correctly refused as already linked. The mock invents an
 * account for any handle, so each run names its own.
 */
async function pickSomebodyNew(page: Page): Promise<string> {
  await expect(page.getByRole("heading", { name: /continue with/i })).toBeVisible();
  const handle = `e2e-${crypto.randomUUID().slice(0, 8)}`;
  await page.getByLabel(/somebody new/i).fill(handle);
  await page.getByRole("button", { name: "Continue" }).click();
  return handle;
}

test("the sign-in page offers the providers that can mint an account", async ({ page }) => {
  await page.goto("/login");
  await expect(page.getByRole("link", { name: /continue with github/i })).toBeVisible();
  await expect(page.getByRole("link", { name: /continue with google/i })).toBeVisible();
  // TONE3000 publishes no verified address, so it can only ever be
  // linked to an account that already exists — offering it here would
  // be a button that always fails.
  await expect(page.getByRole("link", { name: /continue with tone3000/i })).toHaveCount(0);
});

test("signing in with GitHub creates the account and lands on it", async ({ page }) => {
  await page.goto("/login");
  await page.getByRole("link", { name: /continue with github/i }).click();
  await pickOnProvider(page, /Mona Lisa Octocat/i);
  await expect(page).not.toHaveURL(/\/login/);
  await page.goto("/account/profile");
  // GitHub leaves a private address out of the profile call, so this
  // address can only have come from the separate addresses call.
  await expect(sheet(page)).toContainText("octocat@github.local");
});

test("TONE3000 links to an account and the page names the handle", async ({ page }) => {
  await signUpFresh(page);
  await page.goto("/account");
  await sheet(page).getByRole("link", { name: "Link TONE3000" }).click();
  const handle = await pickSomebodyNew(page);
  await expect(page).toHaveURL(/\/account/);
  await expect(sheet(page)).toContainText(`Linked as ${handle}`);
});

test("an unlinked provider can be linked and unlinked again", async ({ page }) => {
  await signUpFresh(page);
  await page.goto("/account");
  await sheet(page).getByRole("link", { name: "Link Google" }).click();
  const handle = await pickSomebodyNew(page);
  await expect(sheet(page)).toContainText(`Linked as ${handle}`);

  await sheet(page).getByRole("button", { name: "Unlink" }).click();
  await expect(sheet(page)).toContainText("Not linked");
});
