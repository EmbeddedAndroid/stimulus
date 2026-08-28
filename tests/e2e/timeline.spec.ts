import { expect, test } from "@playwright/test";

test("timeline acquires and renders a simulated capture", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByText("MagicPort", { exact: true })).toBeVisible();
  await expect(page.locator(".device-pill")).toContainText("sim");
  await expect(page.getByRole("status")).toHaveText(/^(idle|ready)$/);

  await page.getByRole("button", { name: /^▶ Capture$/ }).click();

  await expect(page.locator("canvas.waveform")).toBeVisible();
  await expect(page.locator(".time-ruler")).toBeVisible();
  await expect(page.locator(".time-ruler .ruler-mark.trigger")).toHaveText("T");
  await expect(page.locator(".time-ruler .ruler-tick").first()).toBeVisible();
  await expect(page.locator(".timeline-toolbar strong")).toContainText("Capture");
  await expect(page.locator(".history button").first()).toContainText("samples");

  // View navigation: zoom in shrinks the visible window, fit restores it.
  const span = page.locator(".view-span");
  await expect(span).toBeVisible();
  const full = ((await span.textContent()) ?? "").trim();
  await page.getByTitle("Zoom in (+)").click();
  await expect(span).not.toHaveText(full);
  await page.getByTitle("Fit (0)").click();
  await expect(span).toHaveText(full);

  await expect(page.locator("[role=alert]")).toHaveCount(0);
});

test("acquisition controls are backed by canonical operation ids", async ({ page }) => {
  const operations: string[] = [];
  page.on("request", (request) => {
    const match = new URL(request.url()).pathname.match(/^\/api\/ops\/(.+)$/);
    if (match?.[1] !== undefined) operations.push(match[1]);
  });
  await page.goto("/");
  await page.getByRole("button", { name: /^▶ Capture$/ }).click();
  await expect.poll(() => operations).toContain("acq.single");
  expect(operations).toEqual(expect.arrayContaining(["device.status", "acq.status", "sample.get", "capture.list"]));
});

test("LPF import uses the shared operation and renders its capture", async ({ page }) => {
  const operations: string[] = [];
  page.on("request", (request) => {
    const match = new URL(request.url()).pathname.match(/^\/api\/ops\/(.+)$/);
    if (match?.[1] !== undefined) operations.push(match[1]);
  });
  await page.goto("/");
  await page.getByRole("button", { name: "Import LPF" }).click();
  await expect(page.getByRole("form", { name: "Import LPF project" })).toBeVisible();
  await page.getByRole("button", { name: "Import", exact: true }).click();
  await expect(page.locator("canvas.waveform")).toBeVisible();
  await expect(page.locator(".timeline-toolbar strong")).toContainText("Capture");
  await expect.poll(() => operations).toContain("project.import_lpf");
  await expect(page.locator("[role=alert]")).toHaveCount(0);
});

test("command palette exposes and invokes every canonical operation surface", async ({ page }) => {
  const invoked: string[] = [];
  page.on("request", (request) => {
    const match = new URL(request.url()).pathname.match(/^\/api\/ops\/(.+)$/);
    if (match?.[1] !== undefined) invoked.push(match[1]);
  });
  await page.goto("/");
  await page.getByRole("button", { name: /^Operations/ }).click();
  const dialog = page.getByRole("dialog", { name: "Operations" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole("option")).toHaveCount(459);
  await dialog.getByRole("textbox", { name: "Search operations" }).fill("notes.get");
  await dialog.getByRole("option", { name: /notes\.get/ }).click();
  await dialog.getByRole("button", { name: "Run operation" }).click();
  await expect(dialog).toBeHidden();
  await expect.poll(() => invoked).toContain("notes.get");
  await expect(page.locator("[role=alert]")).toHaveCount(0);
});
