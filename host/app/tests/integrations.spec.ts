import { readFileSync } from "node:fs";
import { expect, test } from "@playwright/test";
import { validationMessage } from "../src/lib/integrations";

// The form's checks before a round trip repeat the host's (host/crates/integrations/src/
// file.rs). Both run the same cases, so the two can't drift apart.

interface Case {
  kind: "obs" | "homeAssistant" | "companion";
  input: Record<string, unknown>;
  saved?: { url: string; password?: string | null; token?: string };
  normalized?: unknown;
  message?: string;
}

const cases: Case[] = JSON.parse(
  readFileSync(new URL("../../crates/integrations/fixtures/validation.json", import.meta.url), "utf8"),
);

test("the form's checks say what the host's say, for every shared case", () => {
  expect(cases.length).toBeGreaterThan(30);
  for (const c of cases) {
    // The window only knows whether a secret is saved, never what it is.
    const saved = c.saved ? { url: c.saved.url, hasPassword: !!c.saved.password, hasToken: !!c.saved.token } : null;
    const got = validationMessage(c.kind, c.input as never, saved);
    expect(got, `${c.kind} ${JSON.stringify(c.input)}`).toBe(c.message ?? null);
  }
});
