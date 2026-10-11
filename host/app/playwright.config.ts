import { defineConfig, devices } from "@playwright/test";

// The UI runs in a plain browser against the mock host (src/lib/mock.ts).
export default defineConfig({
  testDir: "tests",
  use: {
    baseURL: "http://localhost:5174",
    ...devices["Desktop Chrome"],
    viewport: { width: 1000, height: 700 },
    launchOptions: process.env.PW_CHROMIUM ? { executablePath: process.env.PW_CHROMIUM } : {},
  },
  webServer: {
    command: "npx vite --port 5174 --strictPort",
    url: "http://localhost:5174",
    reuseExistingServer: !process.env.CI,
  },
});
