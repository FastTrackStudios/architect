import { PASSWORD, PEOPLE, expect, sheet, signIn, signUpFresh, test } from "./fixtures";

/**
 * The mailed code and link cannot be read from a browser — the dev
 * server logs them rather than sending — so these check what only a
 * browser can: that the alternatives are offered, that the forms lead
 * where they say, and that the code field is one a phone can actually
 * type into. The end-to-end paths are covered in
 * `apps/auth-server/tests/login_surface.rs`, which reads the outbox.
 */
test.describe("ways in", () => {
  test("the sign-in field takes a username, not just an address", async ({ page }) => {
    await page.goto("/login");
    const field = page.getByLabel("Email or username");
    await expect(field).toBeVisible();
    // `type="email"` would have the browser refuse a username before
    // the form was ever submitted.
    await expect(field).toHaveAttribute("type", "text");
  });

  test("a username signs in through the ordinary form", async ({ page }) => {
    const email = await signUpFresh(page);
    const username = `u${Math.random().toString(36).slice(2, 10)}`;
    await page.goto("/account/profile");
    await page.getByLabel("Username").fill(username);
    await page.getByRole("button", { name: /save changes/i }).click();
    await expect(page.getByRole("status")).toBeVisible();

    await page.goto("/account/sessions");
    await sheet(page).getByRole("button", { name: /^sign out$/i }).click();
    await expect(page).toHaveURL(/\/login/);

    await page.getByLabel("Email or username").fill(username);
    await page.getByLabel(/password/i).fill(PASSWORD);
    await page.getByRole("button", { name: "Sign in", exact: true }).click();
    await expect(page).not.toHaveURL(/\/login/);

    await page.goto("/account/profile");
    await expect(sheet(page).getByText(email).first()).toBeVisible();
  });

  test("both passwordless routes are offered from the sign-in page", async ({ page }) => {
    await page.goto("/login");
    await page.getByRole("link", { name: /email me a code/i }).click();
    await expect(page.getByRole("heading", { name: /sign in with a code/i })).toBeVisible();

    await page.getByRole("link", { name: /sign in with a password/i }).click();
    await page.getByRole("link", { name: /email me a link/i }).click();
    await expect(page.getByRole("heading", { name: /sign in with a link/i })).toBeVisible();
  });

  test("asking for a code leads to a field a phone can type into", async ({ page }) => {
    await page.goto("/login/code");
    await page.getByLabel("Email").fill(PEOPLE.ada);
    await page.getByRole("button", { name: /email me a code/i }).click();

    await expect(page.getByText(/a code is on its way/i)).toBeVisible();
    const code = page.getByLabel("Code");
    await expect(code).toBeVisible();
    // The bug this pins: `inputmode="numeric"` shows a number pad, and
    // the code is uppercase base64url — letters, dashes, underscores.
    await expect(code).not.toHaveAttribute("inputmode", "numeric");
    await expect(code).toHaveAttribute("autocomplete", "one-time-code");
  });

  test("a wrong code says so and lets you ask for another", async ({ page }) => {
    await page.goto("/login/code");
    await page.getByLabel("Email").fill(PEOPLE.ada);
    await page.getByRole("button", { name: /email me a code/i }).click();
    await page.getByLabel("Code").fill("000000");
    await page.getByRole("button", { name: "Sign in", exact: true }).click();

    await expect(page.getByRole("alert")).toContainText(/wrong or has expired/i);
    await expect(page.getByRole("button", { name: /send another code/i })).toBeVisible();
  });

  test("asking for a link says the same thing about anybody", async ({ page }) => {
    // A registered address and one that is not: the page must not
    // distinguish them, or it answers "does this person have an
    // account here?" to whoever asks.
    const seen: string[] = [];
    for (const email of [PEOPLE.ada, "definitely-nobody@local.test"]) {
      await page.goto("/login/link");
      await page.getByLabel("Email").fill(email);
      await page.getByRole("button", { name: /email me a link/i }).click();
      await expect(page.getByRole("heading", { name: /sign in with a link/i })).toBeVisible();
      seen.push(
        (await page.getByText(/sign-in link is on its way/i).textContent())?.replace(email, "…") ??
          "",
      );
    }
    expect(seen[0]).toBe(seen[1]);
  });

  test("a link that was already used explains itself", async ({ page }) => {
    await page.goto("/login/magic?email=nobody%40local.test&token=not-a-real-token");
    await expect(page.getByRole("heading", { name: /not usable/i })).toBeVisible();
    // Mail scanners follow links before people do, so the page has to
    // offer a way forward rather than just refusing.
    await expect(page.getByRole("button", { name: /send a new link/i })).toBeVisible();
  });
});
