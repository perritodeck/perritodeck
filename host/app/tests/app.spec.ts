import { expect, test, type Page } from "@playwright/test";

const mock = (page: Page, js: string) => page.evaluate(js);

test("shows the profile as a deck and explains a key", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Buttons" })).toBeVisible();
  await expect(page.getByText("Profile “Starter”, 5 × 3 keys")).toBeVisible();
  // Live clock title is filled in, not shown as {time}.
  await expect(page.getByText("{time}")).toHaveCount(0);

  await page.getByRole("button", { name: /Copy: hotkey Ctrl\+C/ }).click();
  await expect(page.getByRole("heading", { name: "Copy" })).toBeVisible();
  await expect(page.locator("dd", { hasText: "hotkey Ctrl+C" })).toBeVisible();

  // Page keys can be followed.
  await page.getByRole("button", { name: /More ›: opens More/ }).dblclick();
  await expect(page.getByRole("tab", { name: "More" })).toHaveAttribute("aria-selected", "true");
  await expect(page.getByRole("button", { name: /Run a command/ })).toBeVisible();
});

test("reports a broken profile edit and clears it", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Buttons" })).toBeVisible();
  await mock(page, `__perritoMock.setReloadError("default.toml: expected '=' at line 3")`);
  await expect(page.getByText("Your last edit wasn’t applied.")).toBeVisible();
  await expect(page.getByText("expected '=' at line 3")).toBeVisible();
  await mock(page, `__perritoMock.setReloadError(null)`);
  await expect(page.getByText("Your last edit wasn’t applied.")).toHaveCount(0);
});

test("pairs a tablet", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Pair a tablet" }).first().click();
  await expect(page.getByRole("heading", { name: "Pair a tablet" })).toBeVisible();
  await expect(page.getByLabel(/^Pairing code \d( \d){5}$/)).toBeVisible();
  await expect(page.getByText("3FA9 07C2 D41E")).toBeVisible();
  await expect(page.getByText(/works for (4:5\d|5:00)/)).toBeVisible();

  await mock(page, `__perritoMock.hostEvent({kind: "paired", deviceId: "n1", deviceName: "Lenovo Tab"})`);
  await expect(page.getByRole("heading", { name: "Paired with Lenovo Tab" })).toBeVisible();
  await page.getByRole("button", { name: "See tablets" }).click();
  await expect(page.getByText("Lenovo Tab")).toBeVisible();
  await expect(page.getByText("Hasn’t connected yet")).toBeVisible();
});

test("lists and unpairs tablets", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: /^Tablets/ }).click();
  await expect(page.getByText("Fire HD 10")).toBeVisible();
  await expect(page.getByText("Connected now")).toBeVisible();
  await expect(page.getByText("Last seen 3 days ago")).toBeVisible();

  await mock(page, `__perritoMock.hostEvent({kind: "disconnected", deviceId: "a1b2c3"})`);
  await expect(page.getByText("Connected now")).toHaveCount(0);

  const row = page.getByRole("listitem").filter({ hasText: "Pixel Tablet" });
  await row.getByRole("button", { name: "Unpair…" }).click();
  await row.getByRole("button", { name: "Keep" }).click();
  await expect(page.getByText("Pixel Tablet")).toBeVisible();
  await row.getByRole("button", { name: "Unpair…" }).click();
  await row.getByRole("button", { name: "Unpair", exact: true }).click();
  await expect(page.getByText("Pixel Tablet")).toHaveCount(0);
});

test("the tray's Pair item opens the pairing view", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Buttons" })).toBeVisible();
  await mock(page, `__perritoMock.emit("navigate", "pair")`);
  await expect(page.getByRole("heading", { name: "Pair a tablet" })).toBeVisible();
});

test("explains when the host can't start", async ({ page }) => {
  await page.addInitScript(() => {
    const t = setInterval(() => {
      const m = (window as unknown as { __perritoMock?: { failToStart(s: string): void } }).__perritoMock;
      if (m) {
        m.failToStart("could not listen on 0.0.0.0:46710 — is another PerritoDeck host already running?");
        clearInterval(t);
      }
    }, 1);
  });
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "The host didn't start" })).toBeVisible();
  await expect(page.getByText(/another PerritoDeck host already running/)).toBeVisible();
});

test("settings show where things live", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Settings" }).click();
  await expect(page.getByText("192.168.1.20:46710")).toBeVisible();
  await expect(page.getByText("/home/me/.config/PerritoDeck", { exact: true })).toBeVisible();
});
