import { PASSWORD, PEOPLE, expect, sheet, signIn, signUpFresh, test } from "./fixtures";

test.describe("profile", () => {
  test("a saved name comes back on the page it was typed on", async ({ signedIn: page }) => {
    await page.goto("/account/profile");
    await page.getByLabel("Display name").fill("Ada, Countess of Lovelace");
    await page.getByLabel("Username").fill("countess");
    await page.getByRole("button", { name: /save changes/i }).click();

    await expect(page.getByRole("status")).toContainText(/profile saved/i);
    await page.reload();
    await expect(page.getByLabel("Display name")).toHaveValue("Ada, Countess of Lovelace");
    await expect(page.getByLabel("Username")).toHaveValue("countess");
  });

  test("a mistyped confirmation is caught before the password changes", async ({ page }) => {
    // Its own account: this test would otherwise be changing the
    // password of somebody another test is signed in as.
    const email = await signUpFresh(page);
    await page.goto("/account/profile");
    await page.getByLabel("Current password").fill(PASSWORD);
    await page.getByLabel("New password", { exact: true }).fill("a-new-password");
    await page.getByLabel("Confirm new password").fill("a-different-password");
    await page.getByRole("button", { name: /change password/i }).click();

    await expect(page.getByRole("alert")).toContainText(/do not match/i);
    // And the old one still works, which is the assertion that matters.
    await page.goto("/account/sessions");
    await sheet(page).getByRole("button", { name: /^sign out$/i }).click();
    await signIn(page, email);
  });

  test("changing an address says to check the mail, not that it is done", async ({ page }) => {
    // Its own account. With verification off this change applies
    // immediately, so doing it to a shared account renames somebody
    // every other test signs in as — which is exactly how this suite
    // first went red, in whichever test happened to run after it.
    const email = await signUpFresh(page);
    await page.goto("/account/profile");
    await page.getByLabel("New address").fill(`moved-${email}`);
    await page.getByRole("button", { name: /change address/i }).click();

    // Claiming it took effect while it waits on verification is how
    // somebody locks themselves out of their own account.
    await expect(page.getByRole("status")).toContainText(/check the new address/i);
  });
});

test.describe("sessions", () => {
  test("the browser you are reading from is marked, and cannot be revoked by accident", async ({
    signedIn: page,
  }) => {
    await page.goto("/account/sessions");
    const current = page.getByRole("row").filter({ hasText: "This browser" });
    await expect(current).toBeVisible();
    // The row for this session offers "Sign out", never "Revoke" —
    // otherwise the list invites you to cut the branch you are sitting on.
    await expect(current.getByRole("button", { name: "Revoke" })).toBeHidden();
    await expect(current.getByRole("button", { name: /sign out/i })).toBeVisible();
  });

  test("signing in elsewhere adds a row that can be revoked", async ({ page, browser }) => {
    // Its own account, for the same reason: revoking "the first
    // revocable session" must not cut another test's browser off.
    const email = await signUpFresh(page);
    const other = await browser.newContext();
    const otherPage = await other.newPage();
    await signIn(otherPage, email);

    await page.goto("/account/sessions");
    const revocable = page.getByRole("row").filter({ has: page.getByRole("button", { name: "Revoke" }) });
    await expect(revocable.first()).toBeVisible();
    await revocable.first().getByRole("button", { name: "Revoke" }).click();
    await expect(page.getByRole("status")).toContainText(/signed that session out/i);

    // The other browser is now holding a dead token.
    await otherPage.goto("/account/profile");
    await expect(otherPage).toHaveURL(/\/login/);
    await other.close();
  });
});

test.describe("signed out", () => {
  test("every page sends you to sign in, and back again afterwards", async ({ page }) => {
    for (const path of ["/orgs", "/account/profile", "/account/sessions"]) {
      await page.goto(path);
      await expect(page).toHaveURL(/\/login\?return_to=/);
      expect(decodeURIComponent(page.url())).toContain(path);
    }
  });
});

test.describe("two-factor", () => {
  test("enrolling shows a QR, a key and ten codes, and a wrong code does not switch it on", async ({
    page,
  }) => {
    // Its own account: enrolling changes how this person signs in.
    await signUpFresh(page);
    await page.goto("/account/two-factor");
    await expect(page.getByText(/Anyone with your password/)).toBeVisible();

    await page.getByRole("button", { name: /set up two-factor/i }).click();
    await expect(page.getByRole("img", { name: /scan/i })).toBeVisible();
    const key = await page.getByLabel("Setup key").inputValue();
    expect(key.length).toBeGreaterThan(20);
    await expect(page.locator("ul.codes li")).toHaveCount(10);

    await page.getByLabel("Code from your app").fill("000000");
    await page.getByRole("button", { name: /turn on two-factor/i }).click();
    await expect(page.getByRole("alert")).toBeVisible();
    // Still off — a wrong code must not enable it.
    await expect(page.getByText(/Anyone with your password/)).toBeVisible();
  });

  test("the page is reachable from every other account page", async ({ signedIn: page }) => {
    for (const from of ["/account/profile", "/account/sessions", "/account/api-keys", "/orgs"]) {
      await page.goto(from);
      await expect(page.getByRole("link", { name: "Two-factor" })).toBeVisible();
    }
  });
});

test.describe("api keys", () => {
  test("a key is shown once and then only by its prefix", async ({ signedIn: page }) => {
    await page.goto("/account/api-keys");
    await page.getByLabel("Name").fill("Playwright key");
    await page.getByRole("button", { name: /create key/i }).click();

    const key = await page.getByLabel("New API key").inputValue();
    expect(key).toMatch(/^ak_/);

    await page.goto("/account/api-keys");
    // A cell, not any text: "Playwright key" is also sitting in the
    // name field the form remembered.
    await expect(sheet(page).getByRole("cell", { name: "Playwright key" })).toBeVisible();
    await expect(page.getByLabel("New API key")).toBeHidden();
    await expect(page.getByText(key)).toBeHidden();
  });

  test("revoking marks the row and keeps it", async ({ page }) => {
    await signUpFresh(page);
    await page.goto("/account/api-keys");
    await page.getByLabel("Name").fill("Leaked key");
    await page.getByRole("button", { name: /create key/i }).click();

    await page
      .getByRole("row")
      .filter({ hasText: "Leaked key" })
      .getByRole("button", { name: "Revoke" })
      .click();
    await expect(page.getByRole("status")).toContainText(/revoked/i);
    // The record survives the response to a leak.
    await expect(page.getByRole("row").filter({ hasText: "Leaked key" })).toContainText("revoked");
  });
});

test.describe("admin", () => {
  test("a non-administrator is told no, and shown nobody", async ({ page }) => {
    await signIn(page, PEOPLE.grace);
    const response = await page.goto("/admin/users");
    expect(response?.status()).toBe(403);
    await expect(page.getByRole("heading", { name: /not allowed/i })).toBeVisible();
    await expect(sheet(page).getByText(PEOPLE.alan)).toBeHidden();
  });

  test("an administrator sees every account", async ({ signedIn: page }) => {
    // `signedIn` is ada, the seeded administrator.
    await page.goto("/admin/users");
    await expect(page.getByRole("heading", { name: "Users" })).toBeVisible();
    for (const email of Object.values(PEOPLE)) {
      await expect(sheet(page).getByText(email).first()).toBeVisible();
    }
  });
});
