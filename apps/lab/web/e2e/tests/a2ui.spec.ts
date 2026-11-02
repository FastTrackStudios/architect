import { expect, test, type Page } from "@playwright/test";

/**
 * The lab's web host: an agent renders a surface with A2UI_RENDER (more
 * than ten catalog components and one that is not), patches it live, and
 * a press that writes waits for the person's yes in the confirmation
 * sheet. What the person types, ticks, slides and picks lands in the
 * surface's data model (Loro), read back here as the agent would with
 * A2UI_SURFACE.
 */

declare global {
  interface Window {
    a2ui: (id: string, args?: unknown) => Promise<any>;
  }
}

/** Calls an action as the agent, through the page's `window.a2ui`. */
const run = (page: Page, id: string, args?: unknown) =>
  page.evaluate(([id, args]) => window.a2ui(id as string, args), [id, args] as const);

/** The surface's data model, as the agent reads it. */
const data = async (page: Page) => (await run(page, "A2UI_SURFACE", { surfaceId: "form" })).dataModel;

const surface = (page: Page) => page.locator('[data-a2ui-surface="form"]');

test.beforeEach(async ({ page }) => {
  await page.goto("/");
  // The first draw waits on the debug wasm (about 12 MB) being compiled,
  // once per page and in every worker at once: longer than a click.
  await expect(surface(page).getByText("Sign up")).toBeVisible({ timeout: 30_000 });
  await expect(page.locator("[data-lab-error]")).toHaveCount(0);
});

test("the agent's surface is drawn, and what is not in the catalog is not", async ({ page }) => {
  const s = surface(page);
  for (const kind of ["Column", "TextField", "CheckBox", "Slider", "ChoicePicker", "Tabs", "List", "Row"]) {
    await expect(s.locator(`[data-a2ui="${kind}"]`).first()).toBeVisible();
  }
  await expect(s.locator("textarea")).toBeVisible();
  await expect(s.getByText("Ada", { exact: true })).toBeVisible();
  await expect(s.getByText("Second", { exact: true })).toBeVisible();
  await expect(s.getByRole("img", { name: "a dot" })).toBeVisible();
  await expect(s.locator('[data-a2ui-button="save"]')).toBeVisible();
  // The Script is a placeholder: said, never run or fetched.
  await expect(s.locator('[data-a2ui-placeholder="evil"]')).toContainText("not in this app's A2UI catalog");
  await expect(page.locator('script[src*="evil.example"]')).toHaveCount(0);

  const read = await run(page, "A2UI_SURFACE", { surfaceId: "form" });
  expect(read.author).toBe("service:agent");
  expect(read.components.length).toBeGreaterThan(10);
});

test("an agent's patch shows at once", async ({ page }) => {
  await run(page, "A2UI_PATCH", {
    messages: [{ updateDataModel: { surfaceId: "form", path: "/user/name", value: "Grace" } }],
  });
  const s = surface(page);
  await expect(s.getByText("Grace", { exact: true })).toBeVisible();
  await expect(s.getByLabel("Name")).toHaveValue("Grace");
  expect((await data(page)).user.name).toBe("Grace");
});

test("the person's input lands in the data model", async ({ page }) => {
  const s = surface(page);

  await s.getByLabel("Name").fill("Lin");
  await expect(s.getByText("Lin", { exact: true })).toBeVisible();
  await s.getByLabel("About you").fill("Writes compilers.\nPlays cello.");
  await s.getByRole("checkbox").click();
  await expect(s.getByRole("checkbox")).toHaveAttribute("aria-checked", "true");

  const thumb = s.getByRole("slider");
  await thumb.focus();
  // The thumb moves on the arrows: a step, or ten with Shift.
  await page.keyboard.press("Shift+ArrowRight");
  await page.keyboard.press("ArrowRight");
  await expect(thumb).toHaveAttribute("aria-valuenow", "51");
  await expect(s.getByText("Volume: 51")).toBeVisible();

  await s.getByRole("button", { name: "Two", exact: true }).click();
  await expect(s.getByText("Tab two")).toBeVisible();
  await expect(s.getByText("Tab one")).toHaveCount(0);

  await expect.poll(async () => (await data(page)).user).toMatchObject({
    name: "Lin",
    bio: "Writes compilers.\nPlays cello.",
    subscribe: true,
    volume: 51,
    tab: 1,
  });
});

test("a save asks the person, and runs only on their yes", async ({ page }) => {
  const s = surface(page);
  const sheet = page.locator("[data-confirm]");
  await s.getByLabel("Name").fill("Lin");

  await s.getByRole("button", { name: "Save" }).click();
  await expect(sheet).toBeVisible();
  await expect(sheet.getByRole("alertdialog")).toContainText("An agent wants to run “Save the form”");
  await expect(sheet).toContainText('"name": "Lin"');
  await sheet.locator('[data-confirm-answer="deny"]').click();
  await expect(sheet).toHaveCount(0);
  await expect(s.getByText("Not saved yet")).toBeVisible();
  await expect(s.getByText("LAB_SAVE was not approved")).toBeVisible();
  expect((await data(page)).saved).toBe("Not saved yet");
  // A no is said under the button, which can be pressed again.
  await expect(s.getByRole("button", { name: "Save" })).toBeEnabled();

  await s.getByRole("checkbox").click();
  await s.getByRole("button", { name: "Save" }).click();
  await sheet.locator('[data-confirm-answer="allow"]').click();
  await expect(sheet).toHaveCount(0);
  await expect(s.getByText("Saved Lin, subscribed")).toBeVisible();
  expect((await data(page)).saved).toBe("Saved Lin, subscribed");
});

test("the renderer's sheets sit next to the app's own Tailwind", async ({ page }) => {
  // The app's sheet (app.css, Tailwind with its preflight) styles the
  // page around the surface…
  const header = page.locator("[data-app-header]");
  await expect(header).toHaveCSS("background-color", /^(oklch\(0\.457 0\.24 277\.023\)|rgb\(67, 56, 202\))$/);
  await expect(header).toHaveCSS("padding-left", "24px");
  await expect(header.locator("h1")).toHaveCSS("font-weight", "600");

  // …and THEME_CSS and UTILITIES_CSS style the surface: a primary button
  // is the theme's --primary, in the renderer's own utility classes.
  const save = surface(page).getByRole("button", { name: "Save" });
  const primary = await page.evaluate(() => {
    const probe = document.createElement("div");
    probe.style.backgroundColor = "var(--primary)";
    document.body.append(probe);
    const color = getComputedStyle(probe).backgroundColor;
    probe.remove();
    return color;
  });
  expect(primary).not.toBe("rgba(0, 0, 0, 0)");
  await expect(save).toHaveCSS("background-color", primary);
  await expect(save).toHaveCSS("height", "36px");
  // The app's palette is not the renderer's: the header is not --primary.
  expect(await header.evaluate((e) => getComputedStyle(e).backgroundColor)).not.toBe(primary);
});
