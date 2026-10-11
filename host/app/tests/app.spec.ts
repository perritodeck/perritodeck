import { expect, test, type Page } from "@playwright/test";

const mock = (page: Page, js: string) => page.evaluate(js);

test("shows the profile as a deck", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Buttons" })).toBeVisible();
  await expect(page.getByText("Profile “Starter”, 5 × 3 keys")).toBeVisible();
  // Live clock title is filled in, not shown as {time}.
  await expect(page.getByText("{time}")).toHaveCount(0);

  // Page keys can be followed.
  await page.getByRole("button", { name: /More ›: opens More/ }).dblclick();
  await expect(page.getByRole("tab", { name: "More" })).toHaveAttribute("aria-selected", "true");
  await expect(page.getByRole("button", { name: /Run a command/ })).toBeVisible();
});

test("reports a broken profile edit and clears it", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Buttons" })).toBeVisible();
  await mock(page, `__perritoMock.setReloadError("default.toml: expected '=' at line 3")`);
  await expect(page.getByText("Your last edit to the file wasn’t applied.")).toBeVisible();
  await expect(page.getByText("expected '=' at line 3")).toBeVisible();
  await mock(page, `__perritoMock.setReloadError(null)`);
  await expect(page.getByText("Your last edit to the file wasn’t applied.")).toHaveCount(0);
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

const key = (page: Page, name: RegExp | string) => page.getByRole("button", { name });
const lastEdits = (page: Page) => page.evaluate(() => (window as any).__perritoMock.edits.at(-1));

test.describe("editing", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/");
    await expect(page.getByRole("heading", { name: "Buttons" })).toBeVisible();
  });

  test("changes a key's title and action", async ({ page }) => {
    await key(page, /^Copy: hotkey Ctrl\+C/).click();
    await page.getByLabel("Title").fill("Copy all");
    await expect(key(page, /^Copy all: hotkey Ctrl\+C/)).toBeVisible();
    await expect.poll(() => lastEdits(page)).toEqual([
      {
        op: "setKey",
        page: "Main",
        at: [1, 1],
        key: { title: "Copy all", bg: null, fg: null, action: { type: "hotkey", keys: "Ctrl+C" }, nav: null },
      },
    ]);

    await page.getByLabel("When pressed").selectOption("text");
    await expect(page.getByText("Enter the text to type.")).toBeVisible();
    await page.getByLabel("Text to type").fill("hello");
    await expect(key(page, /^Copy all: types "hello"/)).toBeVisible();
  });

  test("records a hotkey", async ({ page }) => {
    await key(page, /^Paste:/).click();
    await page.getByRole("button", { name: "Record" }).click();
    await page.keyboard.press("Control+Shift+KeyK");
    await expect(page.getByRole("textbox", { name: "Keys" })).toHaveValue("Ctrl+Shift+K");
    await expect(key(page, /^Paste: hotkey Ctrl\+Shift\+K/)).toBeVisible();
  });

  test("adds a key in an empty spot, then undo and redo", async ({ page }) => {
    await key(page, "Empty, row 3 column 2").click();
    await page.getByLabel("Title").fill("Stream");
    await page.getByLabel("When pressed").selectOption("obs");
    await page.getByLabel("In OBS").selectOption("toggleStream");
    await expect(key(page, /^Stream: OBS: start or stop streaming/)).toBeVisible();

    // Each change is a step: undo goes back to recording, redo forward again.
    await page.getByRole("button", { name: "Undo" }).click();
    await expect(key(page, /^Stream: OBS: start or stop recording/)).toBeVisible();
    await page.getByRole("button", { name: "Redo" }).click();
    await expect(key(page, /^Stream: OBS: start or stop streaming/)).toBeVisible();
    // Keyboard shortcut (outside a text field) until the key is gone.
    await page.getByRole("heading", { name: "Buttons" }).click();
    await expect
      .poll(async () => {
        await page.keyboard.press("Control+z");
        return key(page, /^Stream/).count();
      })
      .toBe(0);
    await expect(page.getByRole("button", { name: "Undo" })).toBeDisabled();
  });

  test("background colour and reset", async ({ page }) => {
    await key(page, /^Paste:/).click();
    await page.getByLabel("Background").evaluate((el: HTMLInputElement) => {
      el.value = "#ff0000";
      el.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await expect(key(page, /^Paste:/)).toHaveCSS("background-color", "rgb(255, 0, 0)");
    await page.getByRole("button", { name: "Reset" }).click();
    await expect(key(page, /^Paste:/)).toHaveCSS("background-color", "rgb(30, 30, 46)");
  });

  test("moves keys by dragging and with the keyboard", async ({ page }) => {
    await key(page, /^Copy:/).dragTo(key(page, "Empty, row 3 column 3"));
    await expect.poll(() => lastEdits(page)).toEqual([{ op: "moveKey", page: "Main", from: [1, 1], to: [2, 2] }]);
    await expect(key(page, "Empty, row 2 column 2")).toBeVisible();

    // Alt+arrow moves the selected key; it swaps with a neighbour.
    await key(page, /^Copy:/).focus();
    await page.keyboard.press("Alt+ArrowRight");
    await expect.poll(() => lastEdits(page)).toEqual([{ op: "moveKey", page: "Main", from: [2, 2], to: [2, 3] }]);
    await expect(key(page, /^Copy:/)).toBeFocused();

    // Onto a page tab: the key moves to that page.
    await key(page, /^Copy:/).dragTo(page.getByRole("tab", { name: "More" }));
    await expect(page.getByRole("tab", { name: "More" })).toHaveAttribute("aria-selected", "true");
    await expect(key(page, /^Copy:/)).toBeVisible();
  });

  test("removes a key with Delete", async ({ page }) => {
    await key(page, /^Type hello:/).focus();
    await page.keyboard.press("Delete");
    await expect(key(page, /^Type hello/)).toHaveCount(0);
  });

  test("adds, renames, reorders and removes pages", async ({ page }) => {
    await page.getByRole("button", { name: "+ Page" }).click();
    await page.getByLabel("New page name").fill("Stream");
    await page.getByRole("button", { name: "Add page" }).click();
    await expect(page.getByRole("tab", { name: "Stream" })).toHaveAttribute("aria-selected", "true");

    await page.getByRole("button", { name: "Page options" }).click();
    await page.getByRole("menuitem", { name: "Move left" }).click();
    await expect(page.getByRole("tab").nth(1)).toHaveText("Stream");

    await page.getByRole("button", { name: "Page options" }).click();
    await page.getByRole("menuitem", { name: /Rename/ }).click();
    await page.getByLabel("Page name").fill("Live");
    await page.getByRole("button", { name: "Rename" }).click();
    await expect(page.getByRole("tab", { name: "Live" })).toBeVisible();

    await page.getByRole("button", { name: "Page options" }).click();
    await page.getByRole("menuitem", { name: "Remove page" }).click();
    await expect(page.getByRole("tab", { name: "Live" })).toHaveCount(0);

    // A page that a key opens can't be removed; the reason is shown.
    await page.getByRole("tab", { name: "More" }).click();
    await page.getByRole("button", { name: "Page options" }).click();
    await page.getByRole("menuitem", { name: "Remove page" }).click();
    await expect(page.getByRole("alert")).toContainText("opens “More”");
    await expect(page.getByRole("tab", { name: "More" })).toBeVisible();
  });

  test("toggle keys are edited in the file", async ({ page }) => {
    await key(page, /^Mic live/).click();
    await expect(page.getByText(/toggle with 2 states/)).toBeVisible();
    await expect(page.getByLabel("Title")).toBeDisabled();
    await expect(page.getByLabel("When pressed")).toBeDisabled();
  });
});
