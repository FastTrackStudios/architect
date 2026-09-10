import { ORGS, PASSWORD, PEOPLE, expect, sheet, signIn, test } from "./fixtures";

test.describe("organizations", () => {
  test("an owner sees their organizations and can open one", async ({ signedIn: page }) => {
    await page.goto("/orgs");
    await expect(page.getByRole("heading", { name: "Organizations" })).toBeVisible();
    await sheet(page).getByRole("link", { name: new RegExp(ORGS.acme) }).click();

    await expect(page.getByRole("heading", { name: ORGS.acme })).toBeVisible();
    await expect(sheet(page).getByText(PEOPLE.grace)).toBeVisible();
  });

  test("a new organization gets a slug derived from its name", async ({ signedIn: page }) => {
    // Unique per run: slugs are unique server-wide, and the config
    // reuses an already-running server locally, so a fixed name here
    // passes once and then collides forever.
    const suffix = Math.random().toString(36).slice(2, 8);
    const name = `Midnight Sessions, Inc. ${suffix}`;
    await page.goto("/orgs");
    await page.getByLabel("Name").fill(name);
    // Slug left blank on purpose: being made to invent a URL fragment
    // before you can make a workspace is friction for nothing.
    await page.getByRole("button", { name: /create organization/i }).click();

    await expect(page.getByRole("heading", { name })).toBeVisible();
    // Punctuation collapses to single dashes and nothing trails.
    await expect(page.getByText(`/midnight-sessions-inc-${suffix}`)).toBeVisible();
  });

  test("a member sees the roster but none of the levers", async ({ page }) => {
    // Alan is a plain member of Indie Collective.
    await signIn(page, PEOPLE.alan);
    await page.goto("/orgs");
    await sheet(page).getByRole("link", { name: new RegExp(ORGS.indie) }).click();

    await expect(page.getByRole("heading", { name: ORGS.indie })).toBeVisible();
    await expect(sheet(page).getByText(PEOPLE.grace)).toBeVisible();
    // The whole point: what is on the page follows the permission the
    // reader holds, not just what the route allows.
    await expect(page.getByRole("heading", { name: "Invite links" })).toBeHidden();
    await expect(page.getByRole("button", { name: /delete organization/i })).toBeHidden();
  });

  test("a role change shows up in the roster", async ({ signedIn: page }) => {
    await page.goto("/orgs");
    await sheet(page).getByRole("link", { name: new RegExp(ORGS.acme) }).click();

    const row = page.getByRole("row").filter({ hasText: PEOPLE.grace });
    await row.getByRole("combobox").selectOption("member");
    await row.getByRole("button", { name: "Set" }).click();

    await expect(page.getByRole("status")).toContainText(/role updated/i);
    await expect(
      page.getByRole("row").filter({ hasText: PEOPLE.grace }).getByRole("combobox"),
    ).toHaveValue("member");
  });

  test("the last owner cannot leave", async ({ signedIn: page }) => {
    await page.goto("/orgs");
    await sheet(page).getByRole("link", { name: new RegExp(ORGS.acme) }).click();
    await page.getByRole("button", { name: /leave this organization/i }).click();

    // An organization with no owner can be administered by nobody.
    await expect(page.getByRole("alert")).toContainText(/last organization owner/i);
  });
});

test.describe("invite links", () => {
  test("a stranger can see where a link leads before signing up", async ({ signedIn: page, browser }) => {
    await page.goto("/orgs");
    await sheet(page).getByRole("link", { name: new RegExp(ORGS.acme) }).click();

    await page.getByLabel("Label").fill("Playwright cohort");
    await page.getByLabel(/maximum uses/i).fill("1");
    await page.getByRole("button", { name: /create invite link/i }).click();

    // Shown exactly once, because only the hash is stored.
    const url = await page.getByLabel("Invite URL").inputValue();
    expect(url).toMatch(/^\/join\?token=/);

    // A brand-new browser, signed out, following the link.
    const stranger = await browser.newContext();
    const strangerPage = await stranger.newPage();
    await strangerPage.goto(url);
    await expect(strangerPage.getByRole("heading", { name: `Join ${ORGS.acme}` })).toBeVisible();
    await expect(strangerPage.getByRole("button", { name: /sign in and join/i })).toBeVisible();
    await stranger.close();
  });

  test("a revoked link stops admitting between two visits", async ({ signedIn: page }) => {
    await page.goto("/orgs");
    await sheet(page).getByRole("link", { name: new RegExp(ORGS.acme) }).click();
    await page.getByLabel("Label").fill("Revoked shortly");
    await page.getByRole("button", { name: /create invite link/i }).click();

    const url = await page.getByLabel("Invite URL").inputValue();
    await page.goto(url);
    await expect(page.getByRole("heading", { name: `Join ${ORGS.acme}` })).toBeVisible();

    await page.goto("/orgs");
    await sheet(page).getByRole("link", { name: new RegExp(ORGS.acme) }).click();
    await page
      .getByRole("row")
      .filter({ hasText: "Revoked shortly" })
      .getByRole("button", { name: "Revoke" })
      .click();

    await page.goto(url);
    // Bad, spent, revoked and expired all say the same thing: naming
    // which one confirms to a stranger that the token once existed.
    await expect(page.getByRole("heading", { name: /not usable/i })).toBeVisible();
  });

  test("a nonsense token is a dead end, not an error page", async ({ page }) => {
    await page.goto("/join?token=definitely-not-a-real-token");
    await expect(page.getByRole("heading", { name: /not usable/i })).toBeVisible();
  });
});
