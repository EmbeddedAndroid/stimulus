import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: ".",
  timeout: 30_000,
  retries: 0,
  workers: 1,
  expect: {
    // Headless SwiftShader/text antialiasing differs by a handful of sub-pixels
    // run to run (~26px on a 1440x900 page); tolerate that noise so the visual
    // snapshots gate real UI regressions (thousands of pixels), not AA jitter.
    toHaveScreenshot: { maxDiffPixels: 200 },
  },
  reporter: process.env.LP_JUNIT === undefined
    ? "line"
    : [["line"], ["junit", { outputFile: process.env.LP_JUNIT }]],
  use: {
    baseURL: process.env.LP_BASE_URL ?? "http://127.0.0.1:8472",
    headless: true,
    viewport: { width: 1440, height: 900 },
  },
});
