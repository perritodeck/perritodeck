import { expect, test, type Page } from "@playwright/test";

// Settings → Integrations against the mock host (src/lib/mock.ts). Its integrations.toml
// starts with OBS at ws://127.0.0.1:4455 and the password hunter2.

const m = (page: Page) => page.evaluate(() => (window as any).__perritoMock.integrations);
const responses = (page: Page) => page.evaluate(() => JSON.stringify((window as any).__perritoMock.responses));

async function openSettings(page: Page, init?: object) {
  if (init) {
    await page.addInitScript((i) => {
      (window as any).__perritoMockInit = { integrations: i };
    }, init);
  }
  await page.goto("/");
  await page.getByRole("button", { name: "Settings" }).click();
}

const obs = (page: Page) => page.locator("#integrations-obs");
const ha = (page: Page) => page.locator("#integrations-ha");
const companion = (page: Page) => page.locator("#integrations-companion");

test("saved settings show, with the password masked and never sent to the window", async ({ page }) => {
  await openSettings(page);
  const section = obs(page);
  await expect(section.getByLabel("Use OBS Studio")).toBeChecked();
  await expect(section.getByLabel("Address")).toHaveValue("ws://127.0.0.1:4455");
  const password = section.getByLabel("Password");
  await expect(password).toHaveValue("");
  await expect(password).toHaveAttribute("type", "password");
  await expect(password).toHaveAttribute("placeholder", "Saved (type to replace)");
  await expect(section.getByText("A password is saved. Type a new one to replace it.")).toBeVisible();
  // Nothing saved for these: just the box to tick.
  await expect(ha(page).getByLabel("Use Home Assistant")).not.toBeChecked();
  await expect(ha(page).getByLabel("Address")).toHaveCount(0);
  await expect(page.getByText(/readable only by your user account/)).toBeVisible();
  expect(await responses(page)).not.toContain("hunter2");
  // Nothing changed, so nothing to save.
  await expect(section.getByRole("button", { name: "Save", exact: true })).toHaveCount(0);
});

test("Test connection says it's testing, then what it found", async ({ page }) => {
  await openSettings(page);
  await page.evaluate(() => (window as any).__perritoMock.holdTests());
  const section = obs(page);
  const button = section.getByRole("button", { name: "Test connection" });
  await button.click();
  await expect(section.getByRole("button", { name: "Testing…" })).toBeDisabled();
  await page.evaluate(() => (window as any).__perritoMock.releaseTests());
  await expect(section.getByRole("status").filter({ hasText: "Connected to OBS" })).toHaveText(
    "✓ Connected to OBS 31.0.3 (obs-websocket 5.5.0).",
  );
  // The kept password was tested as "keep": the window never had it.
  expect((await m(page)).tests.at(-1)).toEqual({ kind: "obs", url: "ws://127.0.0.1:4455", password: { kind: "keep" } });

  // A wrong password, typed but not saved.
  await section.getByLabel("Password").fill("wrong");
  await expect(section.getByText("Connected to OBS")).toHaveCount(0);
  await button.click();
  const result = section.locator(".try .bad");
  await expect(result).toContainText("OBS rejected the password.");
  expect((await m(page)).tests.at(-1).password).toEqual({ kind: "set", value: "wrong" });
  expect((await m(page)).file.obs.password).toBe("hunter2");
});

test("saving writes the file and offers a restart", async ({ page }) => {
  await openSettings(page);
  const section = ha(page);
  await section.getByLabel("Use Home Assistant").check();
  await section.getByLabel("Address").fill("homeassistant.local:8123");
  await section.getByLabel("Access token").fill("abc");
  await section.getByRole("button", { name: "Save", exact: true }).click();
  await expect(section.getByText("Saved.", { exact: true })).toBeVisible();
  const file = (await m(page)).file;
  expect(file.homeAssistant).toEqual({ enabled: true, url: "http://homeassistant.local:8123", token: "abc" });
  // The saved address shows as the host normalized it; the token stays masked.
  await expect(section.getByLabel("Address")).toHaveValue("http://homeassistant.local:8123");
  await expect(section.getByLabel("Access token")).toHaveValue("");
  await expect(section.getByText("An access token is saved. Type a new one to replace it.")).toBeVisible();
  const restart = page.getByRole("status").filter({ hasText: "PerritoDeck uses the saved settings after it restarts." });
  await expect(restart).toBeVisible();
  await restart.getByRole("button", { name: "Restart now" }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__perritoMock.restarted)).toBe(true);
  expect(await responses(page)).not.toContain('"abc"');
});

test("mistakes are caught before saving or testing", async ({ page }) => {
  await openSettings(page);
  const section = obs(page);
  await section.getByLabel("Address").fill("wss://127.0.0.1:4455");
  await expect(section.getByText("OBS's WebSocket server uses ws://, not wss://.")).toBeVisible();
  await expect(section.getByRole("button", { name: "Save", exact: true })).toBeDisabled();
  await expect(section.getByRole("button", { name: "Test connection" })).toBeDisabled();

  await ha(page).getByLabel("Use Home Assistant").check();
  await ha(page).getByLabel("Address").fill("http://ha.lan:8123");
  await expect(ha(page).getByText("Paste a long-lived access token.")).toBeVisible();

  await companion(page).getByLabel("Use Bitfocus Companion").check();
  await companion(page).getByLabel("Satellite port").fill("70000");
  await expect(companion(page).getByText("Use a number from 1 to 65535.")).toBeVisible();
  await expect(companion(page).getByRole("button", { name: "Save", exact: true })).toBeDisabled();
  expect((await m(page)).saves).toEqual([]);
});

test("turning OBS off keeps its settings", async ({ page }) => {
  await openSettings(page);
  const section = obs(page);
  await section.getByLabel("Use OBS Studio").uncheck();
  await expect(section.getByLabel("Address")).toBeDisabled();
  await section.getByRole("button", { name: "Save", exact: true }).click();
  await expect(section.getByText("Saved.", { exact: true })).toBeVisible();
  expect((await m(page)).file.obs).toEqual({ enabled: false, url: "ws://127.0.0.1:4455", password: "hunter2" });
  await expect(section.getByLabel("Address")).toHaveValue("ws://127.0.0.1:4455");
  await expect(section.getByLabel("Password")).toHaveAttribute("placeholder", "Saved (type to replace)");
  await expect(section.getByRole("button", { name: "Test connection" })).toHaveCount(0);
});

test("removing saved settings asks first", async ({ page }) => {
  await openSettings(page);
  const section = obs(page);
  await section.getByRole("button", { name: "Remove saved settings…" }).click();
  await expect(section.getByText("Removes OBS's address and password from integrations.toml.")).toBeVisible();
  await section.getByRole("button", { name: "Cancel" }).click();
  expect((await m(page)).file.obs).not.toBeNull();
  await section.getByRole("button", { name: "Remove saved settings…" }).click();
  await section.getByRole("button", { name: "Remove", exact: true }).click();
  await expect.poll(async () => (await m(page)).file.obs).toBeNull();
  await expect(section.getByLabel("Use OBS Studio")).not.toBeChecked();
});

test("the saved password can be removed, or kept after all", async ({ page }) => {
  await openSettings(page);
  const section = obs(page);
  await section.getByRole("button", { name: "Remove the saved password" }).click();
  await expect(section.getByLabel("Password")).toBeDisabled();
  await expect(section.getByText("The saved password is removed when you save.")).toBeVisible();
  await section.getByRole("button", { name: "Keep it" }).click();
  await expect(section.getByRole("button", { name: "Save", exact: true })).toHaveCount(0);
  await section.getByRole("button", { name: "Remove the saved password" }).click();
  await section.getByRole("button", { name: "Save", exact: true }).click();
  await expect(section.getByText("Saved.", { exact: true })).toBeVisible();
  expect((await m(page)).file.obs.password).toBeNull();
  await expect(section.getByText("A password is saved.")).toHaveCount(0);
});

test("a kept token for a new address says where it goes", async ({ page }) => {
  await openSettings(page, {
    homeAssistant: { enabled: true, url: "http://homeassistant.local:8123", token: "abc" },
  });
  const section = ha(page);
  await section.getByLabel("Address").fill("http://192.168.1.20:8123");
  await expect(section.getByText("This sends the saved token to 192.168.1.20.")).toBeVisible();
  await section.getByLabel("Address").fill("http://ha.example.com:8123");
  await expect(section.getByText("Type the token again to use it with a new address.")).toBeVisible();
  await expect(section.getByRole("button", { name: "Save", exact: true })).toBeDisabled();
});

test("an address keys can't use gets a suggestion", async ({ page }) => {
  await openSettings(page, { obs: { enabled: true, url: "192.168.1.5", password: null } });
  const section = obs(page);
  await expect(section.getByText("OBS buttons can't use this address. Use ws://192.168.1.5:4455 instead?")).toBeVisible();
  await section.getByRole("button", { name: "Use it" }).click();
  await expect(section.getByLabel("Address")).toHaveValue("ws://192.168.1.5:4455");
  await expect(section.getByRole("button", { name: "Save", exact: true })).toBeEnabled();
});

test("a file with a mistake is shown read-only, with a way out", async ({ page }) => {
  const mistake = "line 2: unknown field `pasword`, expected `url` or `password`";
  await openSettings(page, { mistake });
  await expect(page.getByRole("alert").filter({ hasText: `integrations.toml has a mistake: ${mistake}` })).toBeVisible();
  await expect(page.getByText("OBS, Home Assistant and Companion stay off until it's fixed.")).toBeVisible();
  await expect(obs(page)).toHaveCount(0);
  await page.getByRole("button", { name: "Open integrations.toml" }).click();
  await expect.poll(async () => (await page.evaluate(() => (window as any).__perritoMock.opened)).at(-1)).toBe("integrations.toml");
  // Fixed by hand: Check again shows the form.
  await page.evaluate(() => ((window as any).__perritoMock.integrations.mistake = null));
  await page.getByRole("button", { name: "Check again" }).click();
  await expect(obs(page)).toBeVisible();
});

test("starting over asks first, and keeps the old file", async ({ page }) => {
  await openSettings(page, { mistake: "line 1: expected `.`, `=`" });
  await page.getByRole("button", { name: "Start over…" }).click();
  await expect(page.getByText(/renamed to integrations.toml.old, which keeps your passwords/)).toBeVisible();
  await page.getByRole("button", { name: "Start over", exact: true }).click();
  await expect(obs(page)).toBeVisible();
  const state = await m(page);
  expect(state.startedOver).toBe(true);
  await expect(obs(page).getByLabel("Use OBS Studio")).not.toBeChecked();
});

test("a file others can read can be made private", async ({ page }) => {
  await openSettings(page, { privacy: "readableByOthers" });
  const banner = page.getByRole("alert").filter({ hasText: "Other accounts on this computer can read integrations.toml" });
  await expect(banner).toBeVisible();
  await banner.getByRole("button", { name: "Make it private" }).click();
  await expect(banner).toHaveCount(0);
  await expect(page.getByText(/readable only by your user account/)).toBeVisible();
});

test("a drive without permissions says so", async ({ page }) => {
  await openSettings(page, { privacy: "unknown" });
  await expect(page.getByText(/This drive doesn't keep permissions, so other accounts may be able to read it./)).toBeVisible();
});

test("Set up Companion on a Companion profile lands on its settings", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "+ Profile" }).click();
  await page.getByLabel("New profile name").fill("Show");
  await page.getByLabel("Show Bitfocus Companion’s buttons").check();
  await page.getByRole("button", { name: "Create" }).click();
  await expect(page.getByText("First tell PerritoDeck where Companion runs.")).toBeVisible();
  await page.getByRole("button", { name: "Set up Companion" }).click();
  await expect(page.getByRole("heading", { name: "Settings" })).toBeVisible();
  await expect(companion(page)).toBeInViewport();
  await companion(page).getByLabel("Use Bitfocus Companion").check();
  await expect(companion(page).getByLabel("Computer")).toHaveValue("127.0.0.1");
  await expect(companion(page).getByLabel("Satellite port")).toHaveValue("16622");
  await companion(page).getByRole("button", { name: "Test connection" }).click();
  await expect(companion(page).getByText("✓ Connected to Companion 4.1.0 (Satellite API 1.12.0).")).toBeVisible();
});

test("a Companion profile says when Companion is turned off", async ({ page }) => {
  await page.goto("/");
  await page.evaluate(() => Object.assign((window as any).__perritoMock.hostStatus, { integrationsOff: ["Companion"] }));
  await page.getByRole("button", { name: "+ Profile" }).click();
  await page.getByLabel("New profile name").fill("Show");
  await page.getByLabel("Show Bitfocus Companion’s buttons").check();
  await page.getByRole("button", { name: "Create" }).click();
  await expect(page.getByText("Companion is turned off in Settings → Integrations.")).toBeVisible({ timeout: 6000 });
  await page.getByRole("button", { name: "Open Settings → Integrations" }).click();
  await expect(companion(page)).toBeInViewport();
});
