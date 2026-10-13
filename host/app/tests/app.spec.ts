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

test("says when the firewall may keep tablets on Wi-Fi out", async ({ page }) => {
  const banner = page.getByRole("region", { name: /Wi-Fi/ });
  await page.goto("/");
  await page.getByRole("button", { name: /^Tablets/ }).click();
  await expect(page.getByText("Fire HD 10")).toBeVisible();
  await expect(banner).toHaveCount(0);

  const show = async (networkHint: object) => {
    await page.addInitScript((h) => {
      (window as any).__perritoMockInit = { networkHint: h };
    }, networkHint);
    await page.goto("/");
    await page.getByRole("button", { name: /^Tablets/ }).click();
  };

  // A tablet fell back to the cable, and the firewall has no rule for PerritoDeck.
  await show({ fellBack: ["Fire HD 10"], firewallBlocks: true, publicNetwork: true, port: 46710, os: "windows" });
  await expect(banner).toHaveAccessibleName("Tablets can’t reach this computer over Wi-Fi");
  await expect(banner).toContainText("Fire HD 10 couldn’t connect over Wi-Fi, so it’s using the USB cable.");
  await expect(banner).toContainText("doesn’t let PerritoDeck take connections on this public network.");
  await expect(banner).toContainText("Allow an app through firewall");
  await expect(banner).toContainText("make it private");
  await page.getByRole("button", { name: "Pair a tablet" }).first().click();
  await expect(banner).toBeVisible();

  // Nobody tried yet, on a private network.
  await show({ fellBack: [], firewallBlocks: true, publicNetwork: false, port: 46710, os: "windows" });
  await expect(banner).toHaveAccessibleName("Windows Firewall keeps tablets on Wi-Fi out");
  await expect(banner).toContainText("so only tablets on a USB cable can connect.");
  await expect(banner).toContainText("tick Private");
  await expect(banner).not.toContainText("make it private");

  // Elsewhere it's a port to open.
  await show({ fellBack: ["Fire HD 10", "Pixel Tablet"], firewallBlocks: false, publicNetwork: false, port: 46710, os: "linux" });
  await expect(banner).toContainText("Fire HD 10 and Pixel Tablet couldn’t connect over Wi-Fi, so they’re using the USB cable.");
  await expect(banner).toContainText("on the same network as this computer");
  await expect(banner).toContainText("sudo ufw allow 46710/tcp");
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

test("settings keep the USB line current and say why a tablet can't connect", async ({ page }) => {
  const usb = (change: { usbDevices?: string[]; usbProblems?: string[] }) =>
    page.evaluate((c) => Object.assign((window as any).__perritoMock.hostStatus, c), change);
  await page.goto("/");
  await page.getByRole("button", { name: "Settings" }).click();
  await expect(page.getByText("No tablet on a cable right now")).toBeVisible();
  await usb({ usbProblems: ["G000L40XXXXX: tap Allow on the tablet's \"Allow USB debugging?\" prompt"] });
  await expect(page.getByText("No tablet connected yet")).toBeVisible({ timeout: 6000 });
  await expect(page.getByText(/G000L40XXXXX: tap Allow/)).toBeVisible();
  await usb({ usbProblems: [], usbDevices: ["G000L40XXXXX"] });
  await expect(page.getByText("1 tablet on a cable")).toBeVisible({ timeout: 6000 });
  await expect(page.getByText(/G000L40XXXXX: tap Allow/)).toHaveCount(0);

  // When the host stops answering, the last status stays and nothing goes unhandled.
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await mock(page, `__perritoMock.failToStart("the host stopped")`);
  await page.waitForTimeout(4500);
  await expect(page.getByText("1 tablet on a cable")).toBeVisible();
  expect(errors).toEqual([]);
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
        key: { title: "Copy all", bg: null, fg: null, action: { type: "hotkey", keys: "Ctrl+C" }, nav: null, icon: null, slider: null, states: [], follow: null },
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
    await page.getByLabel("Search icons").fill("micro");
    await page.getByRole("button", { name: "microphone off" }).click();
    await expect(key(page, /^Paste:/).locator("img")).toBeVisible();
    await expect.poll(async () => (await lastEdits(page))?.[0]?.key.icon).toBe("icons/builtin.png");

    // Or a file.
    await page.getByRole("button", { name: "Change…" }).click();
    await page.getByRole("button", { name: "From a file…" }).click();
    await expect.poll(async () => (await lastEdits(page))?.[0]?.key.icon).toBe("icons/mock.png");

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
    await expect.poll(async () => (await lastEdits(page))?.[0]?.key.slider).toEqual({ type: "obs", input: "Mic/Aux" });
    await expect(page.getByRole("button", { name: "Try it" })).toHaveCount(0);
  });

  test("makes and edits a toggle", async ({ page }) => {
    // The starter's mic toggle opens with its two states.
    await key(page, /^Mic live/).click();
    await expect(page.getByRole("radio", { name: /Toggle/ })).toBeChecked();
    await expect(page.getByRole("tab", { name: /Mic\s+live/ })).toBeVisible();
    await page.getByRole("tab", { name: /Mic\s+muted/ }).click();
    await page.getByLabel("State 2 title").fill("Muted!");
    await expect.poll(async () => (await lastEdits(page))?.[0]?.key?.states?.[1]?.title).toBe("Muted!");

    // An ordinary key becomes a toggle that follows OBS.
    await key(page, /^Paste:/).click();
    await page.getByRole("radio", { name: /Toggle/ }).check();
    await page.getByLabel("State 1 title").fill("Rec");
    await page.getByLabel("When pressed").selectOption("obs");
    await page.getByLabel("Follow a live value (optional)").fill("obs:recording");
    await expect
      .poll(async () => {
        const k = (await lastEdits(page))?.[0]?.key;
        return [k?.states?.length, k?.states?.[0]?.title, k?.states?.[0]?.action?.type, k?.follow, k?.action];
      })
      .toEqual([2, "Rec", "obs", "obs:recording", null]);

    // A third state can't be combined with following.
    await page.getByRole("button", { name: "+ State" }).click();
    await expect(page.getByText("A toggle that follows a value needs exactly two states.")).toBeVisible();
  });

  test("several steps as JSON", async ({ page }) => {
    await key(page, "Empty, row 3 column 2").click();
    await page.getByLabel("When pressed").selectOption("multi");
    await page.getByLabel("Steps, as JSON").fill('[{"action": {"type": "text", "text": "hi"}, "delayMs": 50}]');
    await expect
      .poll(async () => (await lastEdits(page))?.[0]?.key.action)
      .toEqual({ type: "multi", steps: [{ action: { type: "text", text: "hi" }, delayMs: 50 }] });
    await page.getByLabel("Steps, as JSON").fill("[nope");
    await expect(page.getByText("The steps aren't valid JSON.")).toBeVisible();
  });

  test("a pasted path loses its quotes", async ({ page }) => {
    // Explorer's "Copy as path" adds them.
    const obs = "C:\\Program Files\\obs-studio\\bin\\64bit\\obs64.exe";
    await key(page, "Empty, row 3 column 2").click();
    await page.getByLabel("When pressed").selectOption("launch");
    await expect(page.getByLabel("Program")).toHaveAttribute("placeholder", obs);
    await page.getByLabel("Program").fill(`"${obs}"`);
    await expect
      .poll(async () => (await lastEdits(page))?.[0]?.key.action)
      .toEqual({ type: "launch", program: obs, args: [] });
    await page.getByLabel("When pressed").selectOption("openPath");
    await page.getByLabel("File or folder").fill(' "C:\\Users\\me\\Music" ');
    await expect
      .poll(async () => (await lastEdits(page))?.[0]?.key.action)
      .toEqual({ type: "openPath", path: "C:\\Users\\me\\Music" });
  });

  test("random action as JSON", async ({ page }) => {
    await key(page, "Empty, row 3 column 2").click();
    await page.getByLabel("When pressed").selectOption("random");
    // Starts from an example that's ready to use.
    await expect.poll(async () => (await lastEdits(page))?.[0]?.key.action?.actions?.length).toBe(2);
    await page.getByLabel("Actions to pick from, as JSON").fill('[{"type": "media", "key": "next"}, {"type": "text", "text": "hi"}]');
    await expect
      .poll(async () => (await lastEdits(page))?.[0]?.key.action)
      .toEqual({ type: "random", actions: [{ type: "media", key: "next" }, { type: "text", text: "hi" }] });
    await expect(key(page, /one of 2 at random/)).toBeVisible();
    await page.getByLabel("Actions to pick from, as JSON").fill("[]");
    await expect(page.getByText("Add at least one action to pick from.")).toBeVisible();
    await page.getByLabel("Actions to pick from, as JSON").fill('[{"text": "no type"}]');
    await expect(page.getByText('Each action needs a "type".')).toBeVisible();
  });

  test("switch profile key", async ({ page }) => {
    await key(page, "Empty, row 3 column 2").click();
    await page.getByLabel("When pressed").selectOption("profile");
    // Another profile is picked to start with, at its first page.
    await expect.poll(async () => (await lastEdits(page))?.[0]?.key.nav).toEqual({ profile: "streaming" });
    await expect(page.getByLabel("Switch to profile")).toHaveValue("streaming");
    const pageIn = page.getByLabel("Page in it");
    await expect(pageIn.locator("option", { hasText: "Main" })).toHaveCount(1);
    await pageIn.selectOption("Main");
    await expect.poll(async () => (await lastEdits(page))?.[0]?.key.nav).toEqual({ profile: "streaming", page: "Main" });
    await pageIn.selectOption("");
    await expect.poll(async () => (await lastEdits(page))?.[0]?.key.nav).toEqual({ profile: "streaming" });

    // Following the key shows that profile, and its Home key comes back.
    await key(page, /switches to streaming/).dblclick();
    await expect(key(page, /^Rec:/)).toBeVisible();
    await key(page, /‹ Home: switches to default/).dblclick();
    await expect(key(page, /^Copy:/)).toBeVisible();
  });

  test("shows what a Stream Deck plugin draws on its key", async ({ page }) => {
    await page.getByRole("tab", { name: "More" }).click();
    // The plugin's image and title, as the tablet shows them, rather than the key's own "Count".
    const counter = key(page, /^3: Stream Deck plugin: com\.example\.counter\.up/);
    await expect(counter.locator("img.live")).toHaveAttribute("src", /^data:image\/svg\+xml/);
    await expect(counter.locator(".live-title")).toHaveText("3");
    await expect(counter).toHaveCSS("background-color", "rgb(0, 0, 0)");

    // It draws again: the preview follows by itself.
    await mock(page, `__perritoMock.drawKey("default", "More", [1, 0], { title: "4" })`);
    const four = key(page, /^4: Stream Deck plugin/);
    await expect(four.locator(".live-title")).toHaveText("4");

    // The key's own settings stay editable, and the inspector says where the look comes from.
    await four.click();
    await expect(page.getByLabel("Title")).toHaveValue("Count");
    await expect(page.getByText("The preview shows what its Stream Deck plugin draws")).toBeVisible();

    // Nothing drawn (no tablet shows the page): the key's own look.
    await mock(page, `__perritoMock.drawKey("default", "More", [1, 0], { image: null, title: null })`);
    const own = key(page, /^Count: Stream Deck plugin/);
    await expect(own).toHaveText("Count");
    await expect(own.locator("img.live")).toHaveCount(0);
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
      { op: "setKey", page: "Main", at: [0, 0], key: { title: "GG", bg: null, fg: null, action: null, nav: null, icon: null, slider: null, states: [], follow: null } },
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

  test("a profile can show Companion's buttons", async ({ page }) => {
    await page.getByRole("button", { name: "+ Profile" }).click();
    await page.getByLabel("New profile name").fill("Show");
    await page.getByLabel("Show Bitfocus Companion’s buttons").check();
    await page.getByRole("button", { name: "Create" }).click();
    await expect(page.getByRole("heading", { name: "Buttons from Bitfocus Companion" })).toBeVisible();
    await expect(page.getByText("From Companion")).toBeVisible();
    await expect(page.getByText("[companion]")).toBeVisible();
    await page.evaluate(() => ((window as any).__perritoMock.hostStatus.companion = "Connected to Companion 4.1.0"));
    await expect(page.getByText("Connected to Companion 4.1.0")).toBeVisible({ timeout: 6000 });
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

  test("network settings are saved and applied after a restart", async ({ page }) => {
    const mock = () => page.evaluate(() => (window as any).__perritoMock);
    await page.getByRole("button", { name: "Settings" }).click();
    const port = page.getByLabel("Port", { exact: true });
    await expect(port).toHaveValue("46710");
    await expect(page.getByRole("button", { name: "Save network settings" })).toHaveCount(0);
    await port.fill("80");
    await expect(page.getByText("Use a number from 1024 to 65535.")).toBeVisible();
    await expect(page.getByRole("button", { name: "Save network settings" })).toBeDisabled();
    await port.fill("47000");
    await page.getByLabel("Let tablets connect over Wi-Fi").uncheck();
    await expect(page.getByText("Only tablets on a USB cable can connect.")).toBeVisible();
    await page.getByRole("button", { name: "Save network settings" }).click();
    await expect(page.getByText("PerritoDeck uses the new settings after it restarts.")).toBeVisible();
    expect((await mock()).network).toEqual({ port: 47000, lan: false });
    await page.getByRole("button", { name: "Restart now" }).click();
    await expect.poll(async () => (await mock()).restarted).toBe(true);
  });

  test("checks for updates on request", async ({ page }) => {
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("button", { name: "Check now" }).click();
    await expect(page.getByText("You have the latest version.")).toBeVisible();
    await page.evaluate(() => ((window as any).__perritoMock.latest = "Couldn't reach GitHub: offline"));
    await page.getByRole("button", { name: "Check now" }).click();
    await expect(page.getByRole("alert")).toContainText("Couldn't reach GitHub");
    await page.evaluate(() => {
      (window as any).__perritoMock.latest = { version: "0.2.0", url: "https://github.com/perritodeck/perritodeck/releases/tag/v0.2.0" };
    });
    await page.getByRole("button", { name: "Check now" }).click();
    await expect(page.getByText("Version 0.2.0 is out.")).toBeVisible();
    await page.getByLabel("Tell me when a new version is out").check();
    await expect.poll(() => page.evaluate(() => (window as any).__perritoMock.checkUpdates)).toBe(true);
  });
});

test.describe("getting help and AI assistants", () => {
  test.beforeEach(async ({ page, context }) => {
    await context.grantPermissions(["clipboard-read", "clipboard-write"]);
    await page.goto("/");
    await page.getByRole("button", { name: "Settings" }).click();
  });
  const mock = (page: Page) => page.evaluate(() => (window as any).__perritoMock);
  const clipboard = (page: Page) => page.evaluate(() => navigator.clipboard.readText());

  test("copies diagnostics, opens the logs folder and turns on detailed logs", async ({ page }) => {
    await expect(page.getByText("/home/me/.config/PerritoDeck/logs")).toBeVisible();
    await page.getByRole("button", { name: "Copy diagnostics" }).click();
    await expect(page.getByRole("status").filter({ hasText: "Copied" })).toBeVisible();
    expect(await clipboard(page)).toContain("PerritoDeck diagnostics");
    await page.getByRole("button", { name: "Open logs folder" }).click();
    await expect.poll(async () => (await mock(page)).opened).toContain("logs");
    await page.getByLabel("Detailed logs").check();
    await expect.poll(async () => (await mock(page)).detailedLogs).toBe(true);
  });

  test("the MCP server is off until turned on, and each permission is separate", async ({ page }) => {
    const enabled = page.getByLabel("Let AI assistants use PerritoDeck");
    await expect(enabled).not.toBeChecked();
    await expect(page.locator(".mcp-state")).toHaveText("Off");
    await expect(page.getByLabel("Press keys")).toBeDisabled();

    await enabled.check();
    await expect(page.getByText("Running at http://127.0.0.1:46711/mcp")).toBeVisible();
    for (const name of ["Press keys", "Edit profiles", "Run actions"]) {
      await expect(page.getByLabel(name)).not.toBeChecked();
    }
    await page.getByLabel("Press keys").check();
    await expect.poll(async () => (await mock(page)).mcpSettings).toMatchObject({ enabled: true, press: true, edit: false, run: false });

    // Running actions comes with a warning.
    await expect(page.getByRole("note")).toHaveCount(0);
    await page.getByLabel("Run actions").check();
    await expect(page.getByRole("note")).toContainText("can do anything you can do on this computer");
    await page.getByLabel("Run actions").uncheck();
    await expect(page.getByRole("note")).toHaveCount(0);

    // The port: checked, and a port in use is reported.
    const port = page.getByLabel("Port for assistants");
    await expect(port).toHaveValue("46711");
    await port.fill("80");
    await expect(page.getByRole("button", { name: "Save port" })).toBeDisabled();
    await port.fill("46799");
    await page.getByRole("button", { name: "Save port" }).click();
    await expect(page.getByText("Couldn’t start: port 46799 is in use by another program")).toBeVisible();
    await port.fill("46710");
    await page.getByRole("button", { name: "Save port" }).click();
    await expect(page.getByRole("alert")).toContainText("the one tablets use");
    await port.fill("46712");
    await page.getByRole("button", { name: "Save port" }).click();
    await expect(page.getByText("Running at http://127.0.0.1:46712/mcp")).toBeVisible();

    await enabled.uncheck();
    await expect(page.locator(".mcp-state")).toHaveText("Off");
  });

  test("copies the snippets and makes a new token", async ({ page }) => {
    await page.getByLabel("Let AI assistants use PerritoDeck").check();
    await expect(page.getByLabel("Settings for apps that start a command")).toContainText('"args": [');
    await page.getByRole("button", { name: "Copy" }).first().click();
    // The bridge in the data folder, not the app itself.
    expect(JSON.parse(await clipboard(page))).toEqual({
      mcpServers: { perritodeck: { command: "/home/me/.config/perritodeck/bin/perritodeck-mcp", args: [] } },
    });
    await expect(page.getByLabel("Claude Code command", { exact: true })).toHaveText(
      'claude mcp add --scope user perritodeck "--" /home/me/.config/perritodeck/bin/perritodeck-mcp',
    );
    await expect(page.getByRole("note")).toHaveCount(0);

    await page.getByText("Connect by address instead").click();
    const http = page.getByLabel("Settings for apps that connect to a URL");
    await expect(http).toContainText("Bearer mocktoken0001");
    await page.getByRole("button", { name: "Make a new token" }).click();
    await expect(page.getByText("New token made.")).toBeVisible();
    await expect(http).toContainText("Bearer mocktoken0002");
    await expect(page.getByLabel("Claude Code command by address")).toContainText("mocktoken0002");
  });

  test("asks to set up again an assistant that starts the app itself", async ({ page }) => {
    await page.addInitScript(() => {
      (window as any).__perritoMockInit = { appBridgeUsed: true };
    });
    await page.goto("/");
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByLabel("Let AI assistants use PerritoDeck").check();
    await expect(page.getByRole("note")).toContainText("older command that starts PerritoDeck itself");
  });
});

test("says when a new version is out, if asked to", async ({ page }) => {
  await page.addInitScript(() => {
    (window as any).__perritoMockInit = {
      checkUpdates: true,
      latest: { version: "0.2.0", url: "https://github.com/perritodeck/perritodeck/releases/tag/v0.2.0" },
    };
  });
  await page.goto("/");
  await expect(page.getByText("PerritoDeck 0.2.0 is out.")).toBeVisible();
  await page.getByRole("button", { name: "See what's new" }).click();
  await expect
    .poll(() => page.evaluate(() => (window as any).__perritoMock.opened))
    .toEqual(["https://github.com/perritodeck/perritodeck/releases/tag/v0.2.0"]);
  await page.getByRole("button", { name: "Not now" }).click();
  await expect(page.getByText("PerritoDeck 0.2.0 is out.")).toHaveCount(0);
});

test.describe("plugins", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/");
    await expect(page.getByRole("heading", { name: "Buttons" })).toBeVisible();
  });

  test("add and remove plugins", async ({ page }) => {
    await page.getByRole("button", { name: "Plugins" }).click();
    await expect(page.getByRole("heading", { name: "Plugins", exact: true })).toBeVisible();
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

  test("plugin settings and live values with an argument", async ({ page }) => {
    await page.getByRole("button", { name: "Plugins" }).click();
    await page.getByRole("button", { name: "Settings for HTTP requests" }).click();
    await page.getByLabel("Base address").fill("https://pve.local:8006");
    await page.getByLabel("Token").fill("s3cret");
    await page.getByLabel("Skip certificate check").check();
    await expect(page.getByLabel("Token")).toHaveAttribute("type", "password");
    await page.getByRole("button", { name: "Save settings" }).click();
    await expect(page.getByText("Saved. The plugin uses the new settings now.")).toBeVisible();
    await expect
      .poll(() => page.evaluate(() => (window as any).__perritoMock.pluginSettings["dev.perritodeck.http"]))
      .toEqual({ base: "https://pve.local:8006", token: "s3cret", insecure: true });

    await page.getByRole("button", { name: "Buttons" }).click();
    await key(page, "Empty, row 3 column 2").click();
    await page.getByLabel("Title").fill("NAS");
    await page.getByRole("button", { name: "Insert a live value…" }).click();
    await page.getByLabel("Live value", { exact: true }).selectOption("plugin:dev.perritodeck.http:up");
    await page.getByLabel("Web address", { exact: true }).fill("https://nas.local");
    await page.getByRole("button", { name: "Insert", exact: true }).click();
    await expect(page.getByLabel("Title")).toHaveValue("NAS {plugin:dev.perritodeck.http:up:https://nas.local}");
    await expect.poll(async () => (await lastEdits(page))?.[0]?.key.title).toBe("NAS {plugin:dev.perritodeck.http:up:https://nas.local}");
  });

  test("adds a Stream Deck plugin and puts its action on a key", async ({ page }) => {
    await page.getByRole("button", { name: "Plugins" }).click();
    const sd = page.locator("section", { has: page.getByRole("heading", { name: "Stream Deck plugins" }) });
    await expect(sd.getByText("also work here")).toBeVisible();
    await sd.getByRole("button", { name: "Add a plugin file…" }).click();
    await expect(sd.getByText("Counter", { exact: true })).toBeVisible();
    await expect(sd.getByText("Running")).toBeVisible();

    await page.getByRole("button", { name: "Buttons" }).click();
    await key(page, "Empty, row 3 column 2").click();
    await page.getByLabel("When pressed").selectOption("sd:com.example.counter/com.example.counter.up");
    await page.getByRole("button", { name: "Open its settings" }).click();
    await expect(page.locator("iframe.sd-page")).toHaveAttribute("src", "about:blank#sd-settings");
    await page.getByRole("button", { name: "Close settings" }).click();
    await expect(page.getByRole("button", { name: "Try it" })).toHaveCount(0);
    await page.getByLabel("Settings, as JSON (optional)").fill("[1]");
    await expect(page.getByText("Settings are a JSON object")).toBeVisible();
    await page.getByLabel("Settings, as JSON (optional)").fill('{"step": 2}');
    await expect
      .poll(async () => (await lastEdits(page))?.[0]?.key.action)
      .toEqual({ type: "sdPlugin", plugin: "com.example.counter", action: "com.example.counter.up", settings: { step: 2 } });

    await page.getByRole("button", { name: "Plugins" }).click();
    await sd.getByRole("button", { name: "Remove Counter…" }).click();
    await sd.getByRole("button", { name: "Remove", exact: true }).click();
    await expect(sd.getByText("Counter", { exact: true })).toHaveCount(0);
  });

  test("a plugin's buttons wait while it stops", async ({ page }) => {
    await page.getByRole("button", { name: "Plugins" }).click();
    const sd = page.locator("section", { has: page.getByRole("heading", { name: "Stream Deck plugins" }) });
    await sd.getByRole("button", { name: "Add a plugin file…" }).click();
    await page.evaluate(() => {
      const m = (window as any).__perritoMock;
      m.stopping = new Promise((done) => (m.letStop = done));
    });
    const restart = sd.getByRole("button", { name: "Restart Counter" });
    await restart.click();
    await expect(restart).toBeDisabled();
    await expect(restart).toHaveText("Restarting…");
    await page.evaluate(() => (window as any).__perritoMock.letStop());
    await expect(restart).toBeEnabled();
    await expect(restart).toHaveText("Restart");
    await expect(page.getByRole("alert")).toHaveCount(0);
  });

  test("adds and updates plugins from the online catalogue", async ({ page }) => {
    await page.getByRole("button", { name: "Plugins" }).click();
    const online = page.locator("section", { has: page.getByRole("heading", { name: "More plugins online" }) });
    await expect(online.getByText("Weather", { exact: true })).toHaveCount(0);
    await online.getByRole("button", { name: "Look online" }).click();
    await expect(online.getByText("Weather", { exact: true })).toBeVisible();
    await online.getByRole("button", { name: "Update from 1.0.0" }).click();
    await expect(online.getByRole("button", { name: /^Update/ })).toHaveCount(0);
    await online.getByRole("button", { name: "Add Weather" }).click();
    await expect(online.getByText("Added")).toHaveCount(2);
    const installed = page.locator("section", { has: page.getByRole("heading", { name: "Installed" }) });
    await expect(installed.getByText("Weather", { exact: true })).toBeVisible();
  });

  test("a slider sends its value to a plugin", async ({ page }) => {
    await key(page, "Empty, row 3 column 2").click();
    await page.getByLabel("Title").fill("Dim");
    // Slider actions aren't offered for keys.
    await expect(page.getByLabel("When pressed").locator('option[value="plugin:dev.perritodeck.http/send-value"]')).toHaveCount(0);
    await page.getByLabel("When pressed").selectOption("slider");
    await page.getByLabel("Slider sets").selectOption("plugin:dev.perritodeck.http/send-value");
    await page.getByLabel("Web address").fill("https://lamp.local/dim?to={value}");
    await page.getByLabel("The slider follows").selectOption("last-status");
    await expect
      .poll(async () => (await lastEdits(page))?.[0]?.key.slider)
      .toEqual({
        type: "plugin",
        plugin: "dev.perritodeck.http",
        action: "send-value",
        settings: { url: "https://lamp.local/dim?to={value}" },
        follow: "last-status",
      });
  });

  test("a key runs a plugin action with its settings", async ({ page }) => {
    await key(page, "Empty, row 3 column 2").click();
    await page.getByLabel("Title").fill("Hook");
    await page.getByLabel("When pressed").selectOption("plugin:dev.perritodeck.http/request");
    await page.getByLabel("Web address").fill("https://example.com/hook");
    await page.getByLabel("Method").selectOption("PUT");
    await page.getByLabel("Skip the check").check();
    await expect
      .poll(async () => (await lastEdits(page))?.[0]?.key.action)
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
