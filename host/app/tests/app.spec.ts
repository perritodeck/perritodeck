import { expect, test, type Page } from "@playwright/test";

const mock = (page: Page, js: string) => page.evaluate(js);

test("shows the profile as a deck", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Buttons" })).toBeVisible();
  await expect(page.getByLabel("Profile", { exact: true })).toHaveValue("default");
  await expect(page.getByText("5 × 3 keys")).toBeVisible();
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
        key: { title: "Copy all", bg: null, fg: null, action: { type: "hotkey", keys: "Ctrl+C" }, nav: null, icon: null, slider: null },
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

  test("tries an action from the editor", async ({ page }) => {
    await page.getByRole("tab", { name: "More" }).click();
    await key(page, /^Run a command:/).click();
    await page.getByRole("button", { name: "Try it" }).click();
    await expect(page.getByText("Done.")).toBeVisible();
    await page.getByLabel("Program").fill("false");
    await page.getByRole("button", { name: "Try it" }).click();
    await expect(page.getByText("Ran, with a warning: exited with status 1")).toBeVisible();
  });

  test("adds an image and copies a key to another spot", async ({ page }) => {
    await key(page, /^Paste:/).click();
    await page.getByRole("button", { name: "Choose…" }).click();
    await expect(key(page, /^Paste:/).locator("img")).toBeVisible();
    await expect.poll(async () => (await lastEdits(page))[0].key.icon).toBe("icons/mock.png");

    await key(page, /^Paste:/).focus();
    await expect(key(page, /^Paste:/)).toBeFocused();
    await page.keyboard.press("Control+c");
    await key(page, "Empty, row 3 column 2").click();
    await page.keyboard.press("Control+v");
    await expect(key(page, /^Paste:/)).toHaveCount(2);
  });

  test("makes a slider", async ({ page }) => {
    await key(page, "Empty, row 3 column 3").click();
    await page.getByLabel("Title").fill("Mic");
    await page.getByLabel("When pressed").selectOption("slider");
    await page.getByLabel("Slider sets").selectOption("obs");
    await expect(page.getByText("Enter the audio source's name as OBS shows it.")).toBeVisible();
    await page.getByLabel("Audio source").fill("Mic/Aux");
    await expect(key(page, /^Mic: slider: OBS volume of Mic\/Aux/)).toBeVisible();
    await expect.poll(async () => (await lastEdits(page))[0].key.slider).toEqual({ type: "obs", input: "Mic/Aux" });
    await expect(page.getByRole("button", { name: "Try it" })).toHaveCount(0);
  });

  test("toggle keys are edited in the file", async ({ page }) => {
    await key(page, /^Mic live/).click();
    await expect(page.getByText(/toggle with 2 states/)).toBeVisible();
    await expect(page.getByLabel("Title")).toBeDisabled();
    await expect(page.getByLabel("When pressed")).toBeDisabled();
  });
});

test.describe("profiles, tablets and switching", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/");
    await expect(page.getByRole("heading", { name: "Buttons" })).toBeVisible();
  });

  test("switch, create, copy, rename and delete profiles", async ({ page }) => {
    const select = page.getByLabel("Profile", { exact: true });
    await select.selectOption("streaming");
    await expect(key(page, /^Rec: OBS/)).toBeVisible();

    await page.getByRole("button", { name: "+ Profile" }).click();
    await page.getByLabel("New profile name").fill("Gaming");
    await page.getByRole("button", { name: "Create" }).click();
    await expect(select).toHaveValue("gaming");
    await expect(key(page, "Empty, row 1 column 1")).toBeVisible();

    // Edits go to the profile on screen.
    await key(page, "Empty, row 1 column 1").click();
    await page.getByLabel("Title").fill("GG");
    await expect.poll(() => lastEdits(page)).toEqual([
      { op: "setKey", page: "Main", at: [0, 0], key: { title: "GG", bg: null, fg: null, action: null, nav: null, icon: null, slider: null } },
    ]);

    await page.getByRole("button", { name: "Profile options" }).click();
    await page.getByRole("menuitem", { name: /Rename/ }).click();
    await page.getByLabel("Profile name").fill("Games");
    await page.getByRole("button", { name: "Rename" }).click();
    await expect(select.locator("option:checked")).toHaveText("Games");

    await page.getByRole("button", { name: "Profile options" }).click();
    await page.getByRole("menuitem", { name: "Make a copy" }).click();
    await page.getByRole("button", { name: "Make copy" }).click();
    await expect(select.locator("option:checked")).toHaveText("Games copy");
    await expect(key(page, /^GG/)).toBeVisible();

    await page.getByRole("button", { name: "Profile options" }).click();
    await page.getByRole("menuitem", { name: "Delete profile" }).click();
    await expect(select).toHaveValue("default");
    await expect(select.locator("option", { hasText: "Games copy" })).toHaveCount(0);
    // The default can't be deleted.
    await page.getByRole("button", { name: "Profile options" }).click();
    await expect(page.getByRole("menuitem", { name: "Delete profile" })).toBeDisabled();
  });

  test("export and import profiles", async ({ page }) => {
    await page.getByRole("button", { name: "Profile options" }).click();
    await page.getByRole("menuitem", { name: "Export…" }).click();
    await expect(page.getByText("Exported “Starter”.")).toBeVisible();
    await page.getByRole("button", { name: "Profile options" }).click();
    await page.getByRole("menuitem", { name: "Import a profile…" }).click();
    await expect(page.getByLabel("Profile", { exact: true })).toHaveValue("imported");
    await expect(page.getByText("Profile imported.")).toBeVisible();
  });

  test("tablets can be renamed and given a profile", async ({ page }) => {
    await page.getByRole("button", { name: /^Tablets/ }).click();
    const row = page.getByRole("listitem").filter({ hasText: "Fire HD 10" });
    await row.getByRole("combobox").selectOption("streaming");
    await expect(row.getByRole("combobox")).toHaveValue("streaming");
    await row.getByRole("button", { name: "Fire HD 10" }).click();
    await page.getByLabel("Tablet name").fill("Desk Fire");
    await page.getByRole("button", { name: "Save" }).click();
    await expect(page.getByRole("button", { name: "Desk Fire" })).toBeVisible();
  });

  test("switching rules", async ({ page }) => {
    await page.getByRole("button", { name: "Switching" }).click();
    await expect(page.getByRole("heading", { name: "Switching" })).toBeVisible();
    await page.getByRole("button", { name: "Add a rule" }).click();
    await expect(page.getByLabel("Switch profiles by app")).toBeChecked();
    await expect(page.getByLabel("App name")).toHaveValue("obs64");
    await expect(page.getByLabel("Profile to show")).toHaveValue("streaming");
    await page.getByLabel("App name").fill("Code");
    await page.getByLabel("App name").blur();
    await expect(page.getByText("Saved")).toBeVisible();
    const saved = await page.evaluate(() => (window as any).__perritoMock.invoke("get_auto_switch"));
    expect(saved).toEqual({ enabled: true, rules: [{ app: "Code", profile: "streaming", device: null }] });
    await page.evaluate(() => (window as any).__perritoMock.emit("host-event", { kind: "foregroundApp", app: "firefox" }));
    await expect(page.getByText("In front just now:")).toContainText("firefox");
    await page.getByRole("button", { name: "Remove" }).click();
    await expect(page.getByLabel("App name")).toHaveCount(0);
  });

  test("macOS asks for the Accessibility permission", async ({ page }) => {
    await page.evaluate(() => {
      (window as any).__perritoMock.accessibility = false;
      window.dispatchEvent(new Event("focus"));
    });
    await expect(page.getByText("Hotkeys and typing need your permission.")).toBeVisible();
    await page.getByRole("button", { name: "Open System Settings" }).click();
    await expect(page.getByText("Hotkeys and typing need your permission.")).toHaveCount(0);
  });

  test("pausing keys shows a banner until resumed", async ({ page }) => {
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByLabel("Pause all keys").check();
    await expect(page.getByText("Keys are paused.")).toBeVisible();
    await page.getByRole("button", { name: "Resume" }).click();
    await expect(page.getByText("Keys are paused.")).toHaveCount(0);
    await expect(page.getByLabel("Pause all keys")).not.toBeChecked();
    await page.getByLabel("Start PerritoDeck when I log in").check();
    await expect.poll(() => page.evaluate(() => (window as any).__perritoMock.autostart)).toBe(true);
  });
});

test.describe("plugins", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/");
    await expect(page.getByRole("heading", { name: "Buttons" })).toBeVisible();
  });

  test("add and remove plugins", async ({ page }) => {
    await page.getByRole("button", { name: "Plugins" }).click();
    await expect(page.getByRole("heading", { name: "Plugins" })).toBeVisible();
    const installed = page.locator("section", { has: page.getByRole("heading", { name: "Installed" }) });
    await expect(installed.getByText("HTTP requests")).toBeVisible();
    await expect(installed.getByText("Running")).toBeVisible();

    const catalog = page.locator("section", { has: page.getByRole("heading", { name: "Comes with PerritoDeck" }) });
    await catalog.getByRole("listitem").filter({ hasText: "MQTT" }).getByRole("button", { name: "Add" }).click();
    await expect(installed.getByText("MQTT", { exact: true })).toBeVisible();

    const row = installed.getByRole("listitem").filter({ hasText: "HTTP requests" });
    await row.getByRole("button", { name: "Remove…" }).click();
    await row.getByRole("button", { name: "Remove", exact: true }).click();
    await expect(installed.getByText("HTTP requests")).toHaveCount(0);
  });

  test("a key runs a plugin action with its settings", async ({ page }) => {
    await key(page, "Empty, row 3 column 2").click();
    await page.getByLabel("Title").fill("Hook");
    await page.getByLabel("When pressed").selectOption("plugin:dev.perritodeck.http/request");
    await page.getByLabel("Web address").fill("https://example.com/hook");
    await page.getByLabel("Method").selectOption("PUT");
    await page.getByLabel("Skip the check").check();
    await expect
      .poll(async () => (await lastEdits(page))[0].key.action)
      .toEqual({
        type: "plugin",
        plugin: "dev.perritodeck.http",
        action: "request",
        settings: { url: "https://example.com/hook", method: "PUT", insecure: true },
      });
    await expect(key(page, /^Hook: dev.perritodeck.http: request/)).toBeVisible();
    await page.getByRole("button", { name: "Try it" }).click();
    await expect(page.getByText("Done.")).toBeVisible();
  });
});
