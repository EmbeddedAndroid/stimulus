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

  // Pin the shown capture (stop following newer captures) so a background poll
  // cannot swap it, and its sample total, during the interactions below.
  await page.locator(".history button").first().click();

  // View navigation: zoom in shrinks the visible window, fit restores it.
  const span = page.locator(".view-span");
  await expect(span).toBeVisible();
  const full = ((await span.textContent()) ?? "").trim();
  await page.getByTitle("Zoom in (+)").click();
  await expect(span).not.toHaveText(full);
  await page.getByTitle("Fit (0)").click();
  await expect(span).toHaveText(full);

  // Cursors: place A (active by default) and B, expect two cursors and a delta.
  const canvas = page.locator("canvas.waveform");
  const box = await canvas.boundingBox();
  const w = box?.width ?? 800;
  await canvas.click({ position: { x: w * 0.3, y: 40 } });
  await page.locator(".cursor-bar").getByRole("button", { name: "B", exact: true }).click();
  await canvas.click({ position: { x: w * 0.6, y: 40 } });
  await expect(page.locator(".cursors-panel .cursor-item")).toHaveCount(2);
  await expect(page.locator(".cursor-delta")).toBeVisible();

  // Measurements: with A and B placed, the A-B rate slot computes a frequency.
  await expect(page.locator(".measure-panel")).toBeVisible();
  await expect(page.locator(".measure-panel")).toContainText(/Hz/);

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

test("acquisition controls drive threshold and pre-trigger operations", async ({ page }) => {
  const operations: string[] = [];
  page.on("request", (request) => {
    const match = new URL(request.url()).pathname.match(/^\/api\/ops\/(.+)$/);
    if (match?.[1] !== undefined) operations.push(match[1]);
  });
  await page.goto("/");
  await page.getByLabel("Pre-trigger buffer").selectOption("35");
  await expect.poll(() => operations).toContain("sample.pretrigger_buffer.set");
  await expect(page.getByLabel("Pre-trigger buffer")).toHaveValue("35");
  await page.getByLabel("Logic threshold").fill("2.5");
  await expect.poll(() => operations).toContain("threshold.set");
  await expect(page.locator("[role=alert]")).toHaveCount(0);
});

test("channel panel toggles visibility and renames signals", async ({ page }) => {
  const operations: string[] = [];
  page.on("request", (request) => {
    const match = new URL(request.url()).pathname.match(/^\/api\/ops\/(.+)$/);
    if (match?.[1] !== undefined) operations.push(match[1]);
  });
  await page.goto("/");
  const row = page.locator(".channel-row").first();
  await row.locator("input.signal-name").fill("PROBE0");
  await row.locator("input.signal-name").press("Tab");
  await expect.poll(() => operations).toContain("signals.rename");
  await row.locator("input.ch-vis").uncheck();
  await expect(row.locator("input.ch-vis")).not.toBeChecked();
  await expect(page.locator("[role=alert]")).toHaveCount(0);
});

test("groups panel creates and lists a signal group", async ({ page }) => {
  const operations: string[] = [];
  page.on("request", (request) => {
    const match = new URL(request.url()).pathname.match(/^\/api\/ops\/(.+)$/);
    if (match?.[1] !== undefined) operations.push(match[1]);
  });
  await page.goto("/");
  await page.getByRole("button", { name: /^▶ Capture$/ }).click();
  const panel = page.locator(".groups-panel");
  await expect(panel).toBeVisible();
  const name = `BUS_${Date.now()}`;
  await panel.getByLabel("Group name").fill(name);
  await panel.getByLabel("Group wires").fill("D0,D1,D2,D3");
  await panel.getByRole("button", { name: "Add" }).click();
  await expect.poll(() => operations).toContain("groups.create");
  await expect(panel.getByText(name)).toBeVisible();
  await expect(page.locator("[role=alert]")).toHaveCount(0);
});

test("state list view shows the capture as a table", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: /^▶ Capture$/ }).click();
  await expect(page.locator("canvas.waveform")).toBeVisible();
  await page.getByRole("button", { name: "States", exact: true }).click();
  await expect(page.locator("table.statelist")).toBeVisible();
  await expect(page.locator("table.statelist tbody tr").first()).toBeVisible();
  await expect(page.locator("table.statelist thead th").first()).toBeVisible();
  await page.getByRole("button", { name: "Waveform", exact: true }).click();
  await expect(page.locator("canvas.waveform")).toBeVisible();
  await expect(page.locator("[role=alert]")).toHaveCount(0);
});

test("decoders panel lists interpreters from an imported project", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Import LPF" }).click();
  const form = page.getByRole("form", { name: "Import LPF project" });
  await form.getByRole("textbox", { name: "LPF path" }).fill(
    "/usr/local/share/logicport/examples/7. I2C, SPI, RS232 Interpreters.LPF",
  );
  await form.getByRole("button", { name: "Import", exact: true }).click();
  await expect(page.locator(".decoders-panel")).toBeVisible();
  await expect(page.locator(".decoders-panel")).toContainText("I2C");
  await expect(page.locator("[role=alert]")).toHaveCount(0);
});

test("decoders panel creates a decoder, decodes it, and removes it", async ({ page }) => {
  const operations: string[] = [];
  page.on("request", (request) => {
    const match = new URL(request.url()).pathname.match(/^\/api\/ops\/(.+)$/);
    if (match?.[1] !== undefined) operations.push(match[1]);
  });
  await page.goto("/");
  await page.getByRole("button", { name: /^▶ Capture$/ }).click();
  await expect(page.locator("canvas.waveform")).toBeVisible();
  const panel = page.locator(".decoders-panel");
  await expect(panel).toBeVisible();

  // Add a decoder pointed at captured channels.
  await panel.getByLabel("Decoder type").selectOption("i2c");
  await panel.getByLabel("Decoder name").fill("BUS_I2C");
  await panel.getByLabel("Decoder wires").fill("D0,D1");
  await panel.getByRole("button", { name: "Add", exact: true }).click();
  await expect.poll(() => operations).toContain("interp.create");
  const item = panel.locator(".decoder-item").filter({ hasText: "BUS_I2C" });
  await expect(item).toHaveCount(1);

  // Decode it against the live capture, then remove it.
  await item.getByRole("button", { name: "Decode" }).click();
  await expect(item.locator(".decoded-frames")).toBeVisible();
  await item.getByRole("button", { name: "✕" }).click();
  await expect.poll(() => operations).toContain("interp.remove");
  await expect(panel.locator(".decoder-item").filter({ hasText: "BUS_I2C" })).toHaveCount(0);
  await expect(page.locator("[role=alert]")).toHaveCount(0);
});

test("decoders panel decodes an imported interpreter", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Import LPF" }).click();
  const form = page.getByRole("form", { name: "Import LPF project" });
  await form.getByRole("textbox", { name: "LPF path" }).fill(
    "/usr/local/share/logicport/examples/7. I2C, SPI, RS232 Interpreters.LPF",
  );
  await form.getByRole("button", { name: "Import", exact: true }).click();
  const panel = page.locator(".decoders-panel");
  await expect(panel).toBeVisible();
  await panel.locator(".decoder-item").first().getByRole("button", { name: "Decode" }).click();
  await expect(panel.locator(".decoded-frames .frame").first()).toBeVisible();
  await expect(page.locator("[role=alert]")).toHaveCount(0);
});
