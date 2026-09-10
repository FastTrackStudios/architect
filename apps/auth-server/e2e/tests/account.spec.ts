import { PASSWORD, PEOPLE, expect, signIn, signUpFresh, test } from "./fixtures";

test.describe("profile", () => {
  test("a saved name comes back on the page it was typed on", async ({ signedIn: page }) => {
    await page.goto("/account/profile");
    await page.getByLabel("Display name").fill("Ada, Countess of Lovelace");
    await page.getByLabel("Username").fill("countess");
    await page.getByRole("button", { name: /save profile/i }).click();

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
    await page.getByRole("button", { name: /^sign out$/i }).click();
    await signIn(page, email);
  });

  test("changing an address says to check the mail, not that it is done", async ({ signedIn: page }) => {
    await page.goto("/account/profile");
    await page.getByLabel("New address").fill("ada+moved@local.test");
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
