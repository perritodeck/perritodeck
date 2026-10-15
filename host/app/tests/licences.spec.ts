import { readFileSync } from "node:fs";
import { expect, test } from "@playwright/test";
import { licencesUrl } from "../src/lib/licences";

// Where Settings points someone whose app couldn't open the licence texts it carries.
// The Playwright server is a dev build, so app.spec.ts only sees the notices on main.

const release = readFileSync(new URL("../../../.github/workflows/release.yml", import.meta.url), "utf8");

test("a release links its own licence texts, a dev build the notices on main", () => {
  const downloads = "https://github.com/perritodeck/perritodeck/releases/download";
  expect(licencesUrl("0.2.0", false)).toBe(`${downloads}/v0.2.0/third-party-licenses.txt`);
  expect(licencesUrl("0.3.0-rc.1", false)).toBe(`${downloads}/v0.3.0-rc.1/third-party-licenses.txt`);
  expect(licencesUrl("0.2.0", true)).toBe("https://github.com/perritodeck/perritodeck/blob/main/THIRD_PARTY_NOTICES.md");
  // What release.yml does: it runs for v1.2.3 tags and attaches dist/third-party-licenses.txt.
  expect(release).toContain('tags: ["v*.*.*"]');
  expect(release).toMatch(/name: release-licences\s+path: dist\/third-party-licenses\.txt\n/);
});

test("the app looks for its licence texts under the name release.yml bundles them as", () => {
  // Only release builds carry the file, so a rename on either side would go unnoticed:
  // every installed app would open the notices on main instead. src-tauri's own tests
  // don't run in CI, so the check is here.
  const lib = readFileSync(new URL("../src-tauri/src/lib.rs", import.meta.url), "utf8");
  const config = release.match(/^\s*licences='([^']+)'/m)?.[1];
  expect(config, "release.yml's --config that bundles the licence texts").toBeDefined();
  const bundled = Object.values(JSON.parse(config!).bundle.resources);
  const opened = lib.match(/^const LICENCES_FILE: &str = "([^"]+)";/m)?.[1];
  expect(opened, "LICENCES_FILE in src-tauri/src/lib.rs").toBeDefined();
  expect(bundled).toContain(opened);
});
