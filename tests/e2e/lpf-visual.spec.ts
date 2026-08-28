import { expect, test } from "@playwright/test";

const examples = [
  "Quickstart.LPF",
  "1. 8051 With XTAL1.LPF",
  "2. 8051 No XTAL1.LPF",
  "3. ADC Sine, Timing mode 500MHz.LPF",
  "4. ADC Sine, State mode 100MHz.LPF",
  "5. USB IO Compression.LPF",
  "6. CNC Serial Port Compression.LPF",
  "7. I2C, SPI, RS232 Interpreters.LPF",
  "8. 125MHz SDRAM Controller.LPF",
  "9. CAN Interpreter - MCP2515 interface.LPF",
  "A. CAN Interpreter - Multiple Frame.LPF",
  "B. PS2 Keyboard Interface.LPF",
  "C. Quad SPI Interface.LPF",
  "D. 1-Wire Interpreter.LPF",
  "E. ISO7815-3 Interpreter - SIM Card.LPF",
  "F. ISO7815-3 Interpreter - Smart Card.LPF",
  "G. I2S Example.LPF",
] as const;

for (const [index, filename] of examples.entries()) {
  test(`LPF visual ${filename}`, async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Import LPF" }).click();
    const form = page.getByRole("form", { name: "Import LPF project" });
    await form.getByRole("textbox", { name: "LPF path" }).fill(
      `/usr/local/share/logicport/examples/${filename}`,
    );
    await form.getByRole("button", { name: "Import", exact: true }).click();
    await expect(form).toBeHidden();
    await expect(page.locator("canvas.waveform")).toBeVisible();
    await expect(page.locator("[role=alert]")).toHaveCount(0);
    await expect(page).toHaveScreenshot(
      `lpf-${String(index).padStart(2, "0")}.png`,
      {
        animations: "disabled",
        fullPage: true,
        mask: [
          page.locator(".timeline-toolbar > div:first-child"),
          page.locator(".history button"),
          // Live telemetry driven by acq.status: the acquisition state (the app
          // itself reports either "idle" or "ready" depending on arm timing),
          // the buffer-fill bar, and the session counters. These change run to
          // run (and constantly on real hardware), so mask them; their
          // behaviour is gated by the timeline e2e + unit tests, not by pixels.
          page.locator(".timeline-toolbar output"),
          page.locator(".buffer"),
          page.locator(".right-panel dl"),
          // Measurement values are computed asynchronously via capture.measure,
          // so whether a value or its loading placeholder is shown depends on
          // fetch timing; mask them. Their correctness is gated by the timeline
          // e2e and the measurement unit tests.
          page.locator(".measure-value"),
          // The WebGL waveform renders with headless SwiftShader, whose
          // sub-pixel antialiasing is not bit-reproducible run to run; mask it
          // so the snapshot gates the deterministic UI (rows, labels, the
          // resistor-code channel swatches). Trace colours are gated by the
          // wireColor unit test instead.
          page.locator("canvas.waveform"),
        ],
      },
    );
  });
}
