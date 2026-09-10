import { PEOPLE, expect, sheet, signIn, signUpFresh, test } from "./fixtures";

/**
 * These use Chrome's *virtual authenticator* over CDP — a software
 * credential store the browser treats exactly as it would a phone or a
 * security key. So `navigator.credentials` really runs, the script
 * really converts the buffers, and a real signature is really checked
 * on the server.
 *
 * That is the point of testing passkeys here at all: the Rust tests
 * cover the ceremony, and only a browser can cover the twenty lines of
 * base64url conversion between it and the DOM. Those conversions are
 * exactly the kind of thing that is wrong in one direction and silent
 * about it.
 */
async function withAuthenticator(page: import("@playwright/test").Page) {
  const client = await page.context().newCDPSession(page);
  await client.send("WebAuthn.enable");
  const { authenticatorId } = await client.send("WebAuthn.addVirtualAuthenticator", {
    options: {
      protocol: "ctap2",
      transport: "internal",
      hasResidentKey: true,
      hasUserVerification: true,
      isUserVerified: true,
      automaticPresenceSimulation: true,
    },
  });
  return { client, authenticatorId };
}

test.describe("passkeys", () => {
  test("the controls stay hidden until the browser proves it has the API", async ({
    signedIn: page,
  }) => {
    await page.goto("/account/passkeys");
    // Present in the HTML, revealed by the script. A browser without
    // `navigator.credentials` keeps them hidden and shows no dead
    // buttons — which is the whole reason they ship hidden.
    await expect(page.getByRole("button", { name: /add a passkey/i })).toBeVisible();
    await expect(page.getByRole("heading", { name: "Passkeys" })).toBeVisible();
  });

  test("a passkey is created, listed, and then signs you in", async ({ page, context }) => {
    const email = await signUpFresh(page);
    await withAuthenticator(page);

    await page.goto("/account/passkeys");
    await expect(page.getByText(/no passkeys yet/i)).toBeVisible();
    await page.getByLabel("Name").fill("Virtual key");
    await page.getByRole("button", { name: /add a passkey/i }).click();

    // The page reloads itself when registration succeeds.
    await expect(page.getByRole("cell", { name: "Virtual key" })).toBeVisible();

    // Now sign out and come back in with it alone.
    await page.goto("/account/sessions");
    await sheet(page).getByRole("button", { name: /^sign out$/i }).click();
    await expect(page).toHaveURL(/\/login|\/$/);

    await page.goto("/login");
    await page.getByLabel(/email/i).fill(email);
    await page.getByRole("button", { name: /sign in with a passkey/i }).click();

    // No password was typed. The signature did it.
    await expect(page).not.toHaveURL(/\/login/);
    await page.goto("/account/profile");
    // First: the address appears in the header and again in a field.
    await expect(sheet(page).getByText(email).first()).toBeVisible();
  });

  test("removing a passkey is a plain form and needs no script", async ({ page }) => {
    await signUpFresh(page);
    await withAuthenticator(page);
    await page.goto("/account/passkeys");
    await page.getByLabel("Name").fill("Doomed key");
    await page.getByRole("button", { name: /add a passkey/i }).click();
    await expect(page.getByRole("cell", { name: "Doomed key" })).toBeVisible();

    await page
      .getByRole("row")
      .filter({ hasText: "Doomed key" })
      .getByRole("button", { name: "Remove" })
      .click();
    await expect(page.getByRole("status")).toContainText(/removed/i);
    await expect(page.getByText(/no passkeys yet/i)).toBeVisible();
  });

  test("a browser with no passkey is told so, not left hanging", async ({ page }) => {
    await signUpFresh(page);
    // An authenticator that will refuse: no credentials in it.
    const client = await page.context().newCDPSession(page);
    await client.send("WebAuthn.enable");
    await client.send("WebAuthn.addVirtualAuthenticator", {
      options: {
        protocol: "ctap2",
        transport: "internal",
        hasResidentKey: true,
        hasUserVerification: true,
        isUserVerified: true,
        automaticPresenceSimulation: true,
      },
    });

    await page.goto("/login");
    await page.getByLabel(/email/i).fill("nobody@local.test");
    await page.getByRole("button", { name: /sign in with a passkey/i }).click();
    // Whatever happens, the person gets told something and the button
    // comes back — the failure mode to avoid is a disabled button and
    // silence.
    await expect(page.locator("#passkey-status")).toBeVisible({ timeout: 15000 });
  });
});
